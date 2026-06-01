// Audio passthrough example for daisy seed
// Currently support for WM8731 codec and PCM3060 codec
// For WM8731 use feature "seed_1_1"
// For PCM3060 use feature "seed_1_2"
//
// Just like they did in https://github.com/zlosynth/daisy
#![no_std]
#![no_main]

use daisy_embassy::{DaisyBoard, hal, led::UserLed, new_daisy_board};
use defmt::{debug, unwrap};
use embassy_executor::Spawner;
use embassy_time::Timer;
use {defmt_rtt as _, panic_probe as _};

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

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    debug!("====program start====");
    let config = daisy_embassy::default_rcc();
    let p = hal::init(config);
    let board: DaisyBoard<'_> = new_daisy_board!(p);

    let led = board.user_led;
    spawner.spawn(blink(led).unwrap());

    let interface = board
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
