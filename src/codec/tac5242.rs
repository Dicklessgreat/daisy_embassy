//! SAI transport for the hardware-controlled Texas Instruments TAC5242 on
//! Daisy Seed3.

use crate::audio::{AudioConfig, AudioIrqs, AudioPeripherals, HALF_DMA_BUFFER_LENGTH};
use defmt::info;
use embassy_stm32::{self as hal, Peri, peripherals, sai};
use embassy_time::Timer;
use hal::peripherals::*;

/// Minimum delay between stable supplies/mode pins and starting ASI clocks.
///
/// The TAC5242 data sheet requires at least 2 ms. Seed3 powers the codec before
/// application startup, but retaining the delay here also makes a warm
/// reinitialization deterministic.
const STARTUP_DELAY_MS: u64 = 2;

/// Number of significant bits in each 32-bit SAI word.
///
/// The TAC5242 transport is configured for full 32-bit samples
/// (`sai::DataSize::Data32` below), so samples must fill the whole word.
pub const SAMPLE_WIDTH_BITS: u32 = 32;

/// Audio codec transport for the hardware-strapped TAC5242 on Seed3.
pub struct Codec<'a> {
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
        info!("set up TAC5242");
        let (sub_block_tx, sub_block_rx) = hal::sai::split_subblocks(p.sai1);

        // Seed3 straps the TAC5242 as a target. SAI1 supplies a 64-bit stereo
        // frame with 32-bit, MSB-first, left-justified samples.
        let mut sai_tx_config = sai::Config::default();
        sai_tx_config.mode = sai::Mode::Master;
        sai_tx_config.tx_rx = sai::TxRx::Transmitter;
        sai_tx_config.sync_output = true;
        sai_tx_config.clock_strobe = sai::ClockStrobe::Falling;
        sai_tx_config.master_clock_divider = audio_config.fs.into_clock_divider();
        sai_tx_config.stereo_mono = sai::StereoMono::Stereo;
        sai_tx_config.data_size = sai::DataSize::Data32;
        sai_tx_config.bit_order = sai::BitOrder::MsbFirst;
        sai_tx_config.frame_sync_polarity = sai::FrameSyncPolarity::ActiveHigh;
        sai_tx_config.frame_sync_offset = sai::FrameSyncOffset::OnFirstBit;
        sai_tx_config.frame_length = 64;
        sai_tx_config.frame_sync_active_level_length = sai::word::U7(32);
        sai_tx_config.fifo_threshold = sai::FifoThreshold::Quarter;

        let mut sai_rx_config = sai_tx_config;
        sai_rx_config.mode = sai::Mode::Slave;
        sai_rx_config.tx_rx = sai::TxRx::Receiver;
        sai_rx_config.sync_input = sai::SyncInput::Internal;
        sai_rx_config.clock_strobe = sai::ClockStrobe::Rising;
        sai_rx_config.sync_output = false;

        let sai_tx = sai::Sai::new_asynchronous_with_mclk(
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
        let sai_rx = sai::Sai::new_synchronous(
            sub_block_rx,
            p.codec_pins.SD_B,
            p.dma1_ch1,
            rx_buffer,
            AudioIrqs,
            sai_rx_config,
        );

        Self {
            sai_tx,
            sai_rx,
            sai_tx_config,
            sai_rx_config,
        }
    }

    pub async fn start(&mut self) -> Result<(), sai::Error> {
        // Keep ASI clocks stopped until the codec's supplies and hardware mode
        // straps have satisfied the data-sheet settling requirement.
        Timer::after_millis(STARTUP_DELAY_MS).await;

        // Starting the master transmitter also clocks the synchronous receiver.
        // Send silence first to avoid an undefined initial DAC frame.
        let write_buf = [0; HALF_DMA_BUFFER_LENGTH];
        self.sai_tx.write(&write_buf).await?;
        self.sai_rx.start()
    }

    pub async fn read(&mut self, read_buf: &mut [u32]) -> Result<(), sai::Error> {
        let mut wire_words = [0; HALF_DMA_BUFFER_LENGTH];
        self.sai_rx.read(&mut wire_words).await?;

        // Seed3's physical TAC5242 receive words are [right, left]. Normalize
        // them at the board boundary so the public callback contract remains
        // [left, right] in both transport directions.
        for (wire, logical) in wire_words.chunks_exact(2).zip(read_buf.chunks_exact_mut(2)) {
            logical[0] = wire[1];
            logical[1] = wire[0];
        }

        Ok(())
    }

    pub async fn write(&mut self, write_buf: &[u32]) -> Result<(), sai::Error> {
        // Seed3's module-level audio contract is [left, right], but physical
        // validation with a stock Daisy Pod showed that consecutive TAC5242
        // transmit words reach [right, left]. Normalize that board-specific
        // wiring here so every audio callback retains the public [left, right]
        // interleaving used by the rest of daisy-embassy.
        let mut wire_words = [0; HALF_DMA_BUFFER_LENGTH];
        for (logical, wire) in write_buf
            .chunks_exact(2)
            .zip(wire_words.chunks_exact_mut(2))
        {
            wire[0] = logical[1];
            wire[1] = logical[0];
        }

        self.sai_tx.write(&wire_words).await
    }
}

#[allow(non_snake_case)]
pub struct Pins<'a> {
    pub MCLK_A: Peri<'a, PE2>,
    pub SCK_A: Peri<'a, PE5>,
    pub FS_A: Peri<'a, PE4>,
    pub SD_A: Peri<'a, PE6>,
    pub SD_B: Peri<'a, PE3>,
}
