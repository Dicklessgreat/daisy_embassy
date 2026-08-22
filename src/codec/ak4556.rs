use embassy_stm32::{
    self as hal, Peri,
    gpio::{Level, Output, Speed},
    peripherals,
};
use hal::peripherals::*;

use embassy_time::Timer;

use crate::audio::{AudioConfig, AudioIrqs, AudioPeripherals, HALF_DMA_BUFFER_LENGTH};
use defmt::info;
use hal::sai::FifoThreshold;
use hal::sai::FrameSyncOffset;
use hal::sai::{BitOrder, SyncInput};

use hal::sai::{self, ClockStrobe, DataSize, FrameSyncPolarity, Mode, StereoMono, TxRx};

/// Number of significant bits in each 32-bit SAI word.
///
/// The AK4556 transport is configured for 24-bit samples
/// (`DataSize::Data24` below), so only the low 24 bits of each word are used.
pub const SAMPLE_WIDTH_BITS: u32 = 24;

pub struct Codec<'a> {
    reset: Output<'a>,
    sai_tx: sai::Sai<'a, peripherals::SAI1, u32>,
    sai_rx: sai::Sai<'a, peripherals::SAI1, u32>,
    pub sai_tx_config: sai::Config,
    pub sai_rx_config: sai::Config,
}

impl<'a> Codec<'a> {
    pub async fn new(
        p: AudioPeripherals<'a>,
        audio_config: AudioConfig,
        tx_buffer: &'a mut [u32],
        rx_buffer: &'a mut [u32],
    ) -> Self {
        info!("set up AK4556");

        let reset = Output::new(p.codec_pins.RESET, Level::High, Speed::Low);

        let (sub_block_tx, sub_block_rx) = hal::sai::split_subblocks(p.sai1);

        info!("set up sai");
        let mut sai_tx_config = sai::Config::default();
        sai_tx_config.mode = Mode::Master;
        sai_tx_config.tx_rx = TxRx::Transmitter;
        sai_tx_config.sync_output = true;
        sai_tx_config.clock_strobe = ClockStrobe::Falling;
        sai_tx_config.master_clock_divider = audio_config.fs.into_clock_divider();
        sai_tx_config.stereo_mono = StereoMono::Stereo;
        sai_tx_config.data_size = DataSize::Data24;
        sai_tx_config.bit_order = BitOrder::MsbFirst;
        sai_tx_config.frame_sync_polarity = FrameSyncPolarity::ActiveHigh;
        sai_tx_config.frame_sync_offset = FrameSyncOffset::OnFirstBit;
        sai_tx_config.frame_length = 64;
        sai_tx_config.frame_sync_active_level_length = embassy_stm32::sai::word::U7(32);
        sai_tx_config.fifo_threshold = FifoThreshold::Quarter;

        let mut sai_rx_config = sai_tx_config;
        sai_rx_config.mode = Mode::Slave;
        sai_rx_config.tx_rx = TxRx::Receiver;
        sai_rx_config.sync_input = SyncInput::Internal;
        sai_rx_config.clock_strobe = ClockStrobe::Rising;
        sai_rx_config.sync_output = false;

        let sai_tx = hal::sai::Sai::new_asynchronous_with_mclk(
            sub_block_tx,
            p.codec_pins.SCK_A,
            p.codec_pins.SD_A,
            p.codec_pins.FS_A,
            p.codec_pins.MCLK_A,
            p.dma1_ch0,
            tx_buffer,
            AudioIrqs,
            sai_tx_config,
        );

        let sai_rx = hal::sai::Sai::new_synchronous(
            sub_block_rx,
            p.codec_pins.SD_B,
            p.dma1_ch1,
            rx_buffer,
            AudioIrqs,
            sai_rx_config,
        );

        Self {
            reset,
            sai_tx,
            sai_rx,
            sai_tx_config,
            sai_rx_config,
        }
    }

    pub async fn start(&mut self) -> Result<(), sai::Error> {
        info!("start AK4556");

        self.reset.set_high();
        Timer::after_millis(1).await;
        self.reset.set_low();
        Timer::after_millis(1).await;
        self.reset.set_high();

        Timer::after_millis(10).await;

        info!("start SAI");
        let write_buf = [0; HALF_DMA_BUFFER_LENGTH];
        self.sai_tx.write(&write_buf).await?;
        self.sai_rx.start()
    }

    pub async fn read(&mut self, read_buf: &mut [u32]) -> Result<(), sai::Error> {
        self.sai_rx.read(read_buf).await
    }

    pub async fn write(&mut self, write_buf: &[u32]) -> Result<(), sai::Error> {
        self.sai_tx.write(write_buf).await
    }
}

#[allow(non_snake_case)]
pub struct Pins<'a> {
    pub MCLK_A: Peri<'a, PE2>,
    pub SCK_A: Peri<'a, PE5>,
    pub FS_A: Peri<'a, PE4>,
    pub SD_A: Peri<'a, PE6>,
    pub SD_B: Peri<'a, PE3>,
    pub RESET: Peri<'a, PB11>,
}
