#![no_std]
#![no_main]

use core::sync::atomic::{AtomicU8, Ordering};

use daisy_embassy::{audio::HALF_DMA_BUFFER_LENGTH, hal, new_daisy_board};
use defmt::{debug, unwrap};
use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_stm32::{bind_interrupts, exti, interrupt};
use embassy_time::Timer;
use hal::gpio::Pull;
use hal::{exti::ExtiInput, gpio::Input};
use {defmt_rtt as _, panic_probe as _};

bind_interrupts!(pub struct Irqs{
    EXTI3 => exti::InterruptHandler<interrupt::typelevel::EXTI3>;
});

#[derive(Clone, Copy)]
enum WaveFrequency {
    High,
    Middle,
    Low,
}

impl WaveFrequency {
    fn as_period(&self) -> u32 {
        match self {
            Self::High => 60,
            Self::Middle => 200,
            Self::Low => 400,
        }
    }
}

impl From<u8> for WaveFrequency {
    fn from(value: u8) -> Self {
        match value {
            0 => WaveFrequency::Low,
            1 => WaveFrequency::Middle,
            2 => WaveFrequency::High,
            _ => panic!(),
        }
    }
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    debug!("====program start====");
    let config = daisy_embassy::default_rcc();
    let p = hal::init(config);
    let board = new_daisy_board!(p);
    let interface = board
        .audio_peripherals
        .prepare_interface(Default::default())
        .await;

    let mute;
    let mut change_freq;

    // Feature flags are needed because of the different pin mappings.
    // change_freq maps to the same underlying MCU pin in both cases
    #[cfg(any(
        feature = "seed",
        feature = "seed_1_1",
        feature = "seed_1_2",
        feature = "seed3"
    ))]
    {
        mute = Input::new(board.pins.d15, Pull::Up);
        change_freq = ExtiInput::new(board.pins.d16, p.EXTI3, Pull::Up, Irqs);
    }

    #[cfg(feature = "patch_sm")]
    {
        mute = Input::new(board.pins.b8, Pull::Up);
        change_freq = ExtiInput::new(board.pins.c5, p.EXTI3, Pull::Up, Irqs);
    }

    let wave_freq = AtomicU8::new(0);

    let change_freq_fut = async {
        let mut local = 0;
        loop {
            change_freq.wait_for_low().await;
            local += 1;
            if local > 2 {
                local = 0;
            }
            wave_freq.store(local, Ordering::SeqCst);
            Timer::after_millis(30).await;
        }
    };

    let mut buf = [0; HALF_DMA_BUFFER_LENGTH];
    let mut smp_pos: u32 = 0;

    let oscillator_fut = async {
        let mut interface = unwrap!(interface.start_interface().await);
        unwrap!(
            interface
                .start_callback(|_input, output| {
                    let period = WaveFrequency::from(wave_freq.load(Ordering::SeqCst)).as_period();
                    for chunk in buf.chunks_mut(2) {
                        let smp = f32_to_u24(make_triangle_wave(smp_pos % period, period));
                        if mute.is_high() {
                            chunk[0] = smp;
                            chunk[1] = smp;
                        } else {
                            //if user push mute button, do not send triangle wave
                            chunk[0] = 0;
                            chunk[1] = 0;
                        }
                        smp_pos = smp_pos.wrapping_add(1);
                    }
                    output.copy_from_slice(&buf);
                })
                .await
        );
    };

    join(change_freq_fut, oscillator_fut).await;
}

fn make_triangle_wave(pos: u32, period_smp: u32) -> f32 {
    assert!(pos <= period_smp);
    if pos <= (period_smp / 2) {
        pos as f32 * 4.0 / period_smp as f32 - 1.0
    } else {
        let pos = pos - period_smp / 2;
        pos as f32 * (-4.0) / period_smp as f32 + 1.0
    }
}

/// convert audio data from f32 to u24
#[inline(always)]
fn f32_to_u24(x: f32) -> u32 {
    //return (int16_t) __SSAT((int32_t) (x * 32767.f), 16);
    let x = x * 8_388_607.0;
    let x = x.clamp(-8_388_608.0, 8_388_607.0);
    (x as i32) as u32
}
