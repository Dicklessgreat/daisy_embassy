use embassy_stm32::{
    self as hal, Peri, peripherals,
    sai::{
        self, BitOrder, ClockStrobe, DataSize, FifoThreshold, FrameSyncOffset, FrameSyncPolarity,
        Mode, StereoMono, SyncInput, TxRx,
    },
    time::Hertz,
};
use hal::peripherals::*;

use defmt::{info, unwrap};
use embassy_time::Timer;

use crate::audio::{AudioConfig, AudioIrqs, AudioPeripherals, Fs};

const I2C_FS: Hertz = Hertz(100_000);

/// Number of significant bits in each 32-bit SAI word.
///
/// The WM8731 transport is configured for 24-bit samples
/// (`DataSize::Data24` below), so only the low 24 bits of each word are used.
pub const SAMPLE_WIDTH_BITS: u32 = 24;

/// A simple HAL for the Cirrus Logic/ Wolfson WM8731 audio codec
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
        let mut sai_rx_config = sai::Config::default();
        sai_rx_config.mode = Mode::Master;
        sai_rx_config.tx_rx = TxRx::Receiver;
        sai_rx_config.sync_output = true;
        sai_rx_config.clock_strobe = ClockStrobe::Falling;
        sai_rx_config.master_clock_divider = audio_config.fs.into_clock_divider();
        sai_rx_config.stereo_mono = StereoMono::Stereo;
        sai_rx_config.data_size = DataSize::Data24;
        sai_rx_config.bit_order = BitOrder::MsbFirst;
        sai_rx_config.frame_sync_polarity = FrameSyncPolarity::ActiveHigh;
        sai_rx_config.frame_sync_offset = FrameSyncOffset::OnFirstBit;
        sai_rx_config.frame_length = 64;
        sai_rx_config.frame_sync_active_level_length = embassy_stm32::sai::word::U7(32);
        sai_rx_config.fifo_threshold = FifoThreshold::Quarter;

        let mut sai_tx_config = sai_rx_config;
        sai_tx_config.mode = Mode::Slave;
        sai_tx_config.tx_rx = TxRx::Transmitter;
        sai_tx_config.sync_input = SyncInput::Internal;
        sai_tx_config.clock_strobe = ClockStrobe::Rising;
        sai_tx_config.sync_output = false;

        let sai_tx = hal::sai::Sai::new_synchronous(
            sub_block_tx,
            p.codec_pins.SD_B,
            p.dma1_ch0,
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
            p.dma1_ch1,
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

        codec.setup_wm8731(audio_config.fs).await;

        codec
    }

    //====================wm8731 register set up functions============================
    async fn setup_wm8731(&mut self, fs: Fs) {
        use wm8731::WM8731;
        info!("setup wm8731 from I2C");

        Timer::after_micros(10).await;

        // reset
        self.write_wm8731_reg(WM8731::reset());
        Timer::after_micros(10).await;

        // wakeup
        self.write_wm8731_reg(WM8731::power_down(|w| {
            Self::final_power_settings(w);
            //output off before start()
            w.output().power_off();
        }));
        Timer::after_micros(10).await;

        // disable input mute, set to 0dB gain
        self.write_wm8731_reg(WM8731::left_line_in(|w| {
            w.both().enable();
            w.mute().disable();
            w.volume().nearest_dB(0);
        }));
        Timer::after_micros(10).await;

        // sidetone off; DAC selected; bypass off; line input selected; mic muted; mic boost off
        self.write_wm8731_reg(WM8731::analog_audio_path(|w| {
            w.sidetone().disable();
            w.dac_select().select();
            w.bypass().disable();
            w.input_select().line_input();
            w.mute_mic().enable();
            w.mic_boost().disable();
        }));
        Timer::after_micros(10).await;

        // disable DAC mute, deemphasis for 48k
        self.write_wm8731_reg(WM8731::digital_audio_path(|w| {
            w.dac_mut().disable();
            w.deemphasis().frequency_48();
        }));
        Timer::after_micros(10).await;

        // nothing inverted, slave, 24-bits, MSB format
        self.write_wm8731_reg(WM8731::digital_audio_interface_format(|w| {
            w.bit_clock_invert().no_invert();
            w.master_slave().slave();
            w.left_right_dac_clock_swap().right_channel_dac_data_right();
            w.left_right_phase().data_when_daclrc_low();
            w.bit_length().bits_24();
            w.format().left_justified();
        }));
        Timer::after_micros(10).await;

        // no clock division, normal mode
        self.write_wm8731_reg(WM8731::sampling(|w| {
            w.core_clock_divider_select().normal();
            w.base_oversampling_rate().normal_256();
            match fs {
                Fs::Fs8000 => {
                    w.sample_rate().adc_8();
                }
                Fs::Fs32000 => {
                    w.sample_rate().adc_32();
                }
                Fs::Fs44100 => {
                    w.sample_rate().adc_441();
                }
                Fs::Fs48000 => {
                    w.sample_rate().adc_48();
                }
                Fs::Fs88200 => {
                    w.sample_rate().adc_882();
                }
                Fs::Fs96000 => {
                    w.sample_rate().adc_96();
                }
            }
            w.usb_normal().normal();
        }));
        Timer::after_micros(10).await;

        // set active
        self.write_wm8731_reg(WM8731::active().active());
        Timer::after_micros(10).await;

        //Note: WM8731's output not yet enabled.
    }

    fn write_wm8731_reg(&mut self, r: wm8731::Register) {
        const AD: u8 = 0x1a; // or 0x1b if CSB is high

        // WM8731 has 16 bits registers.
        // The first 7 bits are for the addresses, and the rest 9 bits are for the "value"s.
        // Let's pack wm8731::Register into 16 bits.
        let byte1: u8 = ((r.address << 1) & 0b1111_1110) | (((r.value >> 8) & 0b0000_0001) as u8);
        let byte2: u8 = (r.value & 0b1111_1111) as u8;
        unwrap!(self.i2c.blocking_write(AD, &[byte1, byte2]));
    }

    fn final_power_settings(w: &mut wm8731::power_down::PowerDown) {
        w.power_off().power_on();
        w.clock_output().power_off();
        w.oscillator().power_off();
        w.output().power_on();
        w.dac().power_on();
        w.adc().power_on();
        w.mic().power_off();
        w.line_input().power_on();
    }

    pub async fn start(&mut self) -> Result<(), sai::Error> {
        info!("start WM8731");
        self.write_wm8731_reg(wm8731::WM8731::power_down(Self::final_power_settings));
        embassy_time::Timer::after_micros(10).await;

        info!("start SAI");
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
    pub SCL: Peri<'a, PH4>,    // I2C SCL
    pub SDA: Peri<'a, PB11>,   // I2C SDA
    pub MCLK_A: Peri<'a, PE2>, // SAI1 MCLK_A
    pub SCK_A: Peri<'a, PE5>,  // SAI1 SCK_A
    pub FS_A: Peri<'a, PE4>,   // SAI1 FS_A
    pub SD_A: Peri<'a, PE6>,   // SAI1 SD_A
    pub SD_B: Peri<'a, PE3>,   // SAI1 SD_B
}
