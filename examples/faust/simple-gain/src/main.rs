// Gain example with faust
// The basic behavior is the same as the "passthrough" example, but provides a demo of driving
// a gain control created with FAUST.
// Connect a tactile switch or similar to the D16 pin, making it go LOW when pressed.
// Each time the button is pressed, the audio being passed through should gradually get quieter
// or gradually get louder.

#![no_std]
#![no_main]
use core::{array::from_fn, num::Wrapping};
use daisy_embassy::{
    DaisyBoard,
    audio::HALF_DMA_BUFFER_LENGTH,
    hal::{self, bind_interrupts, exti::ExtiInput, gpio::Pull, interrupt, mode::Async},
    led::UserLed,
    new_daisy_board,
};
use defmt::{debug, unwrap};
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::Timer;
use faust_ui::{UIRange, UISetAny};
use panic_probe as _;
mod dsp;

static SHARED_VOLUME: Signal<CriticalSectionRawMutex, f32> = Signal::new();

bind_interrupts!(pub struct Irqs{
    EXTI3 => hal::exti::InterruptHandler<interrupt::typelevel::EXTI3>;
});

#[embassy_executor::task]
async fn blink(mut led: UserLed<'static>) {
    // Blink LED while audio passthrough to show sign of life
    loop {
        led.on();
        Timer::after_millis(500).await;

        led.off();
        Timer::after_millis(500).await;
    }
}

#[embassy_executor::task]
async fn handle_gain_button(mut change_gain: ExtiInput<'static, Async>) {
    SHARED_VOLUME.signal(1.0);
    const GAINS: [f32; 10] = [1.0, 0.8, 0.4, 0.2, 0.1, 0.0, 0.1, 0.2, 0.4, 0.8];
    let mut current_index = 0;
    loop {
        change_gain.wait_for_low().await;
        current_index = (current_index + 1) % 10;
        let value = GAINS[current_index];
        defmt::info!("gain button pressed. value: {}", value);
        SHARED_VOLUME.signal(value);
        Timer::after_millis(300).await;
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    debug!("====program start====");
    let config = daisy_embassy::default_rcc();
    let p = hal::init(config);
    let board: DaisyBoard<'_> = new_daisy_board!(p);

    let led = board.user_led;
    spawner.spawn(blink(led).unwrap());
    spawner.spawn(
        handle_gain_button(ExtiInput::new(board.pins.d16, p.EXTI3, Pull::Up, Irqs)).unwrap(),
    );

    let interface = board
        .audio_peripherals
        .prepare_interface(Default::default())
        .await;

    dsp::LpVol::class_init(48000);
    let mut dsp = dsp::LpVol::new();
    dsp.instance_init(48000);
    let mut interface = unwrap!(interface.start_interface().await);
    unwrap!(
        interface
            .start_callback(|input, output| {
                process_audio_faust(&mut dsp, input, output);
            })
            .await
    );
}

fn process_audio_faust(dsp: &mut dsp::LpVol, input: &[u32], output: &mut [u32]) {
    let ibuf: [[f32; 64]; dsp::FAUST_INPUTS] = from_fn(|_| from_fn(|i| u24_to_f32(input[i])));
    let mut obuf: [[f32; 64]; dsp::FAUST_OUTPUTS] = from_fn(|_| [0.0_f32; HALF_DMA_BUFFER_LENGTH]);

    // if a new value is recieved, set it.
    // only checked once per buffer copy
    if let Some(volume) = SHARED_VOLUME.try_take() {
        dsp::UIActive::Gain.set(dsp, dsp::UIActive::Gain.map(volume));
    };

    dsp.compute(HALF_DMA_BUFFER_LENGTH, &ibuf, &mut obuf);

    for (i, f32_value) in obuf[0].iter().enumerate() {
        output[i] = f32_to_u24(*f32_value);
    }
}

// see https://github.com/zlosynth/daisy
// Convert audio PCM data from u24 to f32,
#[inline(always)]
fn u24_to_f32(y: u32) -> f32 {
    let y = (Wrapping(y) + Wrapping(0x0080_0000)).0 & 0x00FF_FFFF; // convert to i32
    (y as f32 / 8_388_608.0) - 1.0 // (2^24) / 2
}

// Convert audio data from f32 to u24 PCM
#[inline(always)]
fn f32_to_u24(x: f32) -> u32 {
    let x = x * 8_388_607.0;
    let x = x.clamp(-8_388_608.0, 8_388_607.0);
    (x as i32) as u32
}
