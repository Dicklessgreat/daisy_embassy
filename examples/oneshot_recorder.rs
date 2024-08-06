//! This example demonstrates recording incoming audio to SD card.
//! By setting the D16 pin low, you can record audio from the SAI input.
//! It triggers to record the audio of next 10 seconds, and automatically stop recording.
//! After it stops recording, it dumps the recorded as WAV file to SD card.
#![no_std]
#![no_main]

use core::sync::atomic::{AtomicU8, Ordering};

use block_device_adapters::BufStream;
use daisy_embassy::{audio::HALF_DMA_BUFFER_LENGTH, hal, new_daisy_board, sdram::SDRAM_SIZE};
use defmt::{debug, info};
use embassy_embedded_hal::shared_bus::asynch::spi::SpiDeviceWithConfig;
use embassy_executor::Spawner;
use embassy_futures::join::join4;
use embassy_stm32::{
    exti::ExtiInput,
    gpio::{self, Pull},
    spi::Spi,
    time::Hertz,
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex, signal::Signal};
use embassy_time::{Delay, Timer};
use embedded_fatfs::FsOptions;
use embedded_hal_async::delay::DelayNs;
use grounded::uninit::GroundedArrayCell;
use sdspi::SdSpi;
use {defmt_rtt as _, panic_probe as _};

//DMA buffer must be in special region. Refer https://embassy.dev/book/#_stm32_bdma_only_working_out_of_some_ram_regions
#[link_section = ".sram1_bss"]
static mut STORAGE: GroundedArrayCell<u8, 512> = GroundedArrayCell::uninit();

struct RecordingHasFinished;
//48000(Hz) * 10(Sec) * 2(stereo)
const RECORD_LENGTH: usize = 960_000;
const SILENCE: u32 = u32::MAX / 2;
static STATE: AtomicU8 = AtomicU8::new(0);
const IDLE: u8 = 0;
const RECORDING: u8 = 1;
const FLUSHING: u8 = 2;
static RECORDING_HAS_FINISHED: Signal<CriticalSectionRawMutex, RecordingHasFinished> =
    Signal::new();

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    debug!("====program start====");
    let config = daisy_embassy::default_rcc();
    let p = hal::init(config);
    let mut c = cortex_m::Peripherals::take().unwrap();
    let board = new_daisy_board!(p);
    let (mut interface, (mut to_interface, mut from_interface)) = board
        .audio_peripherals
        .prepare_interface(Default::default())
        .await;
    let sdram: Mutex<CriticalSectionRawMutex, _> =
        Mutex::new(board.sdram.build(&mut c.MPU, &mut c.SCB));
    let led: Mutex<CriticalSectionRawMutex, _> = Mutex::new(board.user_led);

    let audio_callback_fut = async {
        // Block Length
        const BL: usize = HALF_DMA_BUFFER_LENGTH;
        //record point
        let mut rp = 0;
        loop {
            let rx = from_interface.receive().await;
            // if triggered record, record incoming buffer till RECORD_LENGTH
            if STATE.load(Ordering::SeqCst) == RECORDING {
                // The only time SDRAM is used elsewhere is when flushing recorded sound,
                // and flushing only happens when the recording has been finished.
                // When it fails to acquire lock and some audio samples are missed,
                // let's take that it is not a "failure".
                if let Ok(mut sdram) = sdram.try_lock() {
                    let mut delay = Delay;
                    let loop_buffer = unsafe {
                        // Initialise controller and SDRAM
                        let ram_ptr: *mut u32 = sdram.init(&mut delay) as *mut _;

                        // Convert raw pointer to slice
                        core::slice::from_raw_parts_mut(
                            ram_ptr,
                            SDRAM_SIZE / core::mem::size_of::<u32>(),
                        )
                    };

                    let remain = BL.min(RECORD_LENGTH - rp);
                    loop_buffer[rp..(rp + remain)].copy_from_slice(rx);
                    rp += BL;
                    if rp >= RECORD_LENGTH {
                        rp = 0;
                        STATE.store(FLUSHING, Ordering::SeqCst);
                        RECORDING_HAS_FINISHED.signal(RecordingHasFinished);
                        info!("finished recording");
                    }
                }
            }
            from_interface.receive_done();

            //clear DA buffer
            to_interface
                .send()
                .await
                .iter_mut()
                .for_each(|smp| *smp = SILENCE);
            to_interface.send_done();
        }
    };

    let mut record_pin = ExtiInput::new(board.pins.d16, p.EXTI3, Pull::Up);
    let record_fut = async {
        loop {
            record_pin.wait_for_low().await;
            if STATE
                .compare_exchange(IDLE, RECORDING, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                led.lock().await.on();
                info!("record!!");
            };
            Timer::after_secs(10).await;
        }
    };
    let mut spi_config = embassy_stm32::spi::Config::default();
    // frequency must be low during card initialization
    spi_config.frequency = Hertz::khz(400);
    let spi: Mutex<CriticalSectionRawMutex, _> = Mutex::new(Spi::new(
        p.SPI1,
        board.pins.d8,
        board.pins.d10,
        board.pins.d9,
        p.DMA1_CH0,
        p.DMA1_CH3,
        spi_config,
    ));

    let spi = SpiDeviceWithConfig::new(
        &spi,
        gpio::Output::new(board.pins.d7, gpio::Level::High, gpio::Speed::High),
        spi_config,
    );

    let buf: &mut [u8] = unsafe {
        STORAGE.initialize_all_copied(0);
        let (ptr, len) = STORAGE.get_ptr_len();
        core::slice::from_raw_parts_mut(ptr, len)
    };
    let mut sd = SdSpi::<_, _, aligned::A1>::new(spi, Delay, buf);

    loop {
        // Initialize the card
        match sd.init().await {
            Ok(_) => {
                // Increase the speed up to 15mhz
                let mut config = embassy_stm32::spi::Config::default();
                config.frequency = Hertz::mhz(15);
                sd.spi().set_config(config);
                defmt::info!("Initialization complete!");

                break;
            }
            Err(e) => {
                info!("{:?}", defmt::Debug2Format(&e));
            }
        }
        defmt::info!("Failed to init card, retrying...");
        Delay.delay_ns(5000u32).await;
    }

    let inner = BufStream::<_, 512>::new(sd);
    let fs = embedded_fatfs::FileSystem::new(inner, FsOptions::new())
        .await
        .unwrap();
    let root = fs.root_dir();
    let mut file = root.create_file("recorded.wav").await.unwrap();
    // dump the recorded to SD card
    let dump_fut = async {
        use embedded_io_async::Write;
        loop {
            RECORDING_HAS_FINISHED.wait().await;
            info!("flush the recorded to SD card");
            let mut sdram = sdram.lock().await;
            let mut delay = Delay;
            let loop_buffer = unsafe {
                // Initialise controller and SDRAM
                let ram_ptr: *mut u32 = sdram.init(&mut delay) as *mut _;

                // Convert raw pointer to slice
                core::slice::from_raw_parts_mut(ram_ptr, SDRAM_SIZE / core::mem::size_of::<u32>())
            };
            file.write(&wav_header()).await.unwrap();
            for chunk in loop_buffer[..RECORD_LENGTH].chunks(1 << 10) {
                let mut tmp = [0; (1 << 10) * 3];
                for (i, smp) in chunk.iter().enumerate() {
                    let bytes = smp.to_le_bytes();
                    tmp[i * 3] = bytes[0];
                    tmp[i * 3 + 1] = bytes[1];
                    tmp[i * 3 + 2] = bytes[2];
                }
                file.write(&tmp).await.unwrap();
            }
            file.flush().await.unwrap();
            info!("finish flushing to sd card!!");
            STATE.store(IDLE, Ordering::SeqCst);
            led.lock().await.off();
        }
    };
    join4(interface.start(), audio_callback_fut, record_fut, dump_fut).await;
}

/// 24bit, 48KHz canonical wav container header
fn wav_header() -> [u8; 44] {
    let mut result = [0; 44];
    //Note chunk sizes are little endian
    //RIFF chunk
    result[0] = b'R';
    result[1] = b'I';
    result[2] = b'F';
    result[3] = b'F';
    //RIFF chunk size(2880000 + 36 == 0x002bf224)
    result[4] = 0x24;
    result[5] = 0xf2;
    result[6] = 0x2b;
    result[7] = 0x00;
    //WAVE identifier
    result[8] = b'W';
    result[9] = b'A';
    result[10] = b'V';
    result[11] = b'E';
    //fmt chunk
    result[12] = b'f';
    result[13] = b'm';
    result[14] = b't';
    result[15] = b' ';
    //fmt size
    result[16] = 0x10;
    result[17] = 0x00;
    result[18] = 0x00;
    result[19] = 0x00;
    //fmt content
    //format(1 == pcm)
    result[20] = 1;
    result[21] = 0;
    //num channels
    result[22] = 2;
    result[23] = 0;
    //sampling rate(48000 == 0x0000bb80)
    result[24] = 0x80;
    result[25] = 0xbb;
    result[26] = 0;
    result[27] = 0;
    //bytes per sec
    result[28] = 0x00;
    result[29] = 0x65;
    result[30] = 0x04;
    result[31] = 0x00;
    //I forgot what this is
    result[32] = 0x04;
    result[33] = 0x00;
    // bits per sample(0x0018 == 24bit)
    result[34] = 0x18;
    result[35] = 0x00;
    //data chunk
    result[36] = b'd';
    result[37] = b'a';
    result[38] = b't';
    result[39] = b'a';
    //data size(960000 * 24 / 8 = 2880000 == 0x002bf200)
    result[40] = 0x00;
    result[41] = 0xf2;
    result[42] = 0x2b;
    result[43] = 0x00;
    result
}
