// Controls the speed of the blinking onboard LED using a potentiometer.
// It also acts as an audio passthrough at the same time to demonstrate the fact the DMM is used
// Connect the middle pin/wiper of the potentiometer to pin ADC1/D16,
// connect bottom pin to ground, and top pin to 3V3 Analogue(pin 21).

#![no_std]
#![no_main]

use daisy_embassy::led::UserLed;
use daisy_embassy::new_daisy_board;
use defmt::{info, unwrap};
use embassy_executor::Spawner;
use embassy_stm32::adc::{Adc, AdcChannel as _, AnyAdcChannel, SampleTime};
use embassy_stm32::peripherals::ADC1;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::Timer;
use grounded::uninit::GroundedArrayCell;
use {defmt_rtt as _, panic_probe as _};

use {defmt_rtt as _, panic_probe as _};

static PERIOD_CONTROL: Signal<CriticalSectionRawMutex, u16> = Signal::new();

#[unsafe(link_section = ".sram1_bss")]
static ADC_BUFFER: GroundedArrayCell<u16, 2> = GroundedArrayCell::uninit();

#[embassy_executor::task]
async fn blink(mut led: UserLed<'static>) {
    // Blink LED while audio passthrough to show sign of life
    let mut period = 100;
    loop {
        info!("on");
        led.on();
        Timer::after_millis(period).await;

        info!("off");
        led.off();
        Timer::after_millis(period).await;

        if let Some(measured) = PERIOD_CONTROL.try_take() {
            period = (measured / 256 + 100) as u64;
        }
    }
}

#[embassy_executor::task]
async fn read_adc(
    mut adc: Adc<'static, ADC1>,
    mut vrefint_channel: AnyAdcChannel<ADC1>,
    mut pc0: AnyAdcChannel<ADC1>,
    mut p_dma2_ch1: embassy_stm32::peripherals::DMA2_CH1,
) {
    // let mut read_buffer: [u16; 2] = [0; 2];
    let adc_buffer: &mut [u16] = unsafe {
        ADC_BUFFER.initialize_all_copied(0);
        let (ptr, len) = ADC_BUFFER.get_ptr_len();
        core::slice::from_raw_parts_mut(ptr, len)
    };

    const UPDATE_PERIOD_MS: u64 = 100;
    loop {
        Timer::after_millis(UPDATE_PERIOD_MS).await;
        adc.read(
            &mut p_dma2_ch1,
            [
                (&mut vrefint_channel, SampleTime::CYCLES387_5),
                (&mut pc0, SampleTime::CYCLES387_5),
            ]
            .into_iter(),
            adc_buffer,
        )
        .await;

        let vrefint = adc_buffer[0];
        info!("vrefint: {}", vrefint);
        let measured = adc_buffer[1];
        info!("measured: {}", measured);
        PERIOD_CONTROL.signal(measured);
    }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let mut config = daisy_embassy::default_rcc();
    config.rcc.mux.adcsel = embassy_stm32::rcc::mux::Adcsel::PLL3_R;

    let p: embassy_stm32::Peripherals = embassy_stm32::init(config);
    let daisy_p = new_daisy_board!(p);
    spawner.spawn(blink(daisy_p.user_led)).unwrap();

    let adc: Adc<'static, ADC1> = Adc::new(p.ADC1);
    let vrefint_channel: AnyAdcChannel<ADC1> = adc.enable_vrefint().degrade_adc();
    let pc0: AnyAdcChannel<ADC1> = daisy_p.pins.d16.degrade_adc();

    spawner
        .spawn(read_adc(adc, vrefint_channel, pc0, p.DMA2_CH1))
        .unwrap();

    // Audio passthrouth to demostrate the fact that this is infact using DMM
    let interface = daisy_p
        .audio_peripherals
        .prepare_interface(Default::default())
        .await;

    let mut interface = unwrap!(interface.start_interface().await);
    unwrap!(
        interface
            .start_callback(|input, output| {
                output.copy_from_slice(input);
            })
            .await
    );
}
