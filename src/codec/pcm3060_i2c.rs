//! A simple HAL for the Texas Instruments PCM3060 audio codec
use crate::audio::{AudioConfig, AudioIrqs, AudioPeripherals, HALF_DMA_BUFFER_LENGTH};
use embassy_stm32::Peri;
use embassy_stm32::{self as hal, peripherals, sai, time::Hertz};
use hal::peripherals::*;

use defmt::{info, unwrap};
use embassy_time::Timer;

const I2C_FS: Hertz = Hertz(100_000);

// PCM3060 I2C constants
const I2C_CODEC_ADDRESS: u8 = 0x8c >> 1;

// PCM3060 register addresses
const SYS_CTRL_REGISTER: u8 = 0x40; // 64
const ADC_CTRL1_REGISTER: u8 = 0x48; // 72
const DAC_CTRL1_REGISTER: u8 = 0x43; // 67

// PCM3060 register masks
const MRST_MASK: u8 = 0x80;
const SRST_MASK: u8 = 0x40;
const ADC_PSV_MASK: u8 = 0x20;
const DAC_PSV_MASK: u8 = 0x10;
const FMT_MASK: u8 = 0x1;

/// Number of significant bits in each 32-bit SAI word.
///
/// The PCM3060 transport is configured for 24-bit samples
/// (`sai::DataSize::Data24` below), so only the low 24 bits of each word are used.
pub const SAMPLE_WIDTH_BITS: u32 = 24;

pub struct Codec<'a> {
    i2c: hal::i2c::I2c<'a, hal::mode::Blocking, hal::i2c::Master>,
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
        info!("set up i2c");
        let mut i2c_config = hal::i2c::Config::default();
        i2c_config.frequency = I2C_FS;
        let i2c = embassy_stm32::i2c::I2c::new_blocking(
            p.i2c2,
            p.codec_pins.SCL,
            p.codec_pins.SDA,
            i2c_config,
        );

        info!("set up sai");
        let (sub_block_rx, sub_block_tx) = hal::sai::split_subblocks(p.sai1);

        // The configuration was made to match the the register values obtained with
        // https://github.com/zlosynth/daisy
        let mut sai_rx_config = hal::sai::Config::default();
        sai_rx_config.mode = sai::Mode::Master;
        sai_rx_config.tx_rx = sai::TxRx::Receiver;
        sai_rx_config.sync_output = true;
        sai_rx_config.clock_strobe = sai::ClockStrobe::Rising;
        sai_rx_config.master_clock_divider = audio_config.fs.into_clock_divider();
        sai_rx_config.stereo_mono = sai::StereoMono::Stereo;
        sai_rx_config.data_size = sai::DataSize::Data24;
        sai_rx_config.bit_order = sai::BitOrder::MsbFirst;
        sai_rx_config.frame_sync_polarity = sai::FrameSyncPolarity::ActiveLow;
        sai_rx_config.frame_sync_offset = sai::FrameSyncOffset::OnFirstBit;
        sai_rx_config.frame_length = 64;
        sai_rx_config.frame_sync_active_level_length = sai::word::U7(32);
        sai_rx_config.fifo_threshold = sai::FifoThreshold::Quarter;
        sai_rx_config.mute_detection_counter = hal::dma::word::U5(0);
        sai_rx_config.slot_size = sai::SlotSize::Channel32;
        sai_rx_config.complement_format = sai::ComplementFormat::OnesComplement;

        let mut sai_tx_config = sai_rx_config;
        sai_tx_config.mode = sai::Mode::Slave;
        sai_tx_config.tx_rx = sai::TxRx::Transmitter;
        sai_tx_config.sync_input = sai::SyncInput::Internal;
        sai_tx_config.clock_strobe = sai::ClockStrobe::Rising;
        sai_rx_config.frame_sync_polarity = sai::FrameSyncPolarity::ActiveHigh;
        sai_tx_config.sync_output = false;

        let sai_tx = hal::sai::Sai::new_synchronous(
            sub_block_tx,
            p.codec_pins.SD_B,
            p.dma1_ch1,
            tx_buffer,
            AudioIrqs,
            sai_tx_config,
        );

        let sai_rx = hal::sai::Sai::new_asynchronous_with_mclk(
            sub_block_rx,
            p.codec_pins.SCK_A,
            p.codec_pins.SD_A,
            p.codec_pins.FS_A,
            p.codec_pins.MCLK_A,
            p.dma1_ch0,
            rx_buffer,
            AudioIrqs,
            sai_rx_config,
        );

        let mut codec = Self {
            i2c,
            sai_tx,
            sai_rx,
            sai_tx_config,
            sai_rx_config,
        };

        info!("set up PCM3060 i2c");
        codec.setup_pcm3060().await;

        codec
    }

    async fn setup_pcm3060(&mut self) {
        // Reset codec
        self.write_pcm3060_reg(SYS_CTRL_REGISTER, MRST_MASK, false)
            .await;
        self.write_pcm3060_reg(SYS_CTRL_REGISTER, SRST_MASK, false)
            .await;

        // Set 24-bit Left-Justified ft
        self.write_pcm3060_reg(ADC_CTRL1_REGISTER, FMT_MASK, true)
            .await;
        self.write_pcm3060_reg(DAC_CTRL1_REGISTER, FMT_MASK, true)
            .await;

        // Disable power saving
        self.write_pcm3060_reg(SYS_CTRL_REGISTER, ADC_PSV_MASK, false)
            .await;
        self.write_pcm3060_reg(SYS_CTRL_REGISTER, DAC_PSV_MASK, false)
            .await;
    }

    async fn write_pcm3060_reg(&mut self, register: u8, mask: u8, set: bool) {
        // Read current register value
        let mut buffer = [0];
        unwrap!(
            self.i2c
                .blocking_write_read(I2C_CODEC_ADDRESS, &[register], &mut buffer)
        );

        // Modify value based on mask and set flag
        let value = if set {
            buffer[0] | mask
        } else {
            buffer[0] & !mask
        };

        // Write back modified value
        unwrap!(
            self.i2c
                .blocking_write(I2C_CODEC_ADDRESS, &[register, value])
        );

        Timer::after_micros(10).await;
    }

    pub async fn start(&mut self) -> Result<(), sai::Error> {
        info!("start SAI");

        let write_buf = [0; HALF_DMA_BUFFER_LENGTH];
        self.sai_tx.write(&write_buf).await?;
        self.sai_rx.start()
    }

    pub fn release(
        self,
    ) -> (
        sai::Sai<'a, SAI1, u32>,
        sai::Sai<'a, SAI1, u32>,
        hal::i2c::I2c<'a, hal::mode::Blocking, hal::i2c::Master>,
    ) {
        (self.sai_tx, self.sai_rx, self.i2c)
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
    pub SCL: Peri<'a, PB10>,   // I2C2 SCL
    pub SDA: Peri<'a, PB11>,   // I2C2 SDA
    pub MCLK_A: Peri<'a, PE2>, // SAI1 MCLK_A
    pub SCK_A: Peri<'a, PE5>,  // SAI1 SCK_A
    pub FS_A: Peri<'a, PE4>,   // SAI1 FS_A
    pub SD_A: Peri<'a, PE6>,   // SAI1 SD_A
    pub SD_B: Peri<'a, PE3>,   // SAI1 SD_B
}
