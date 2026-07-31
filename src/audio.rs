use core::convert::Infallible;
use core::marker::PhantomData;

use crate::codec::{Codec, Pins as CodecPins};
use defmt::info;
use embassy_stm32::{self as hal, Peri, bind_interrupts, dma};
use grounded::uninit::GroundedArrayCell;

use hal::sai::{self, MasterClockDivider};

// - global constants ---------------------------------------------------------

pub const BLOCK_LENGTH: usize = 32; // 32 samples
pub const HALF_DMA_BUFFER_LENGTH: usize = BLOCK_LENGTH * 2; //  2 channels
pub const DMA_BUFFER_LENGTH: usize = HALF_DMA_BUFFER_LENGTH * 2; //  2 half-blocks

// - static data --------------------------------------------------------------

//DMA buffer must be in special region. Refer https://embassy.dev/book/#_stm32_bdma_only_working_out_of_some_ram_regions
#[unsafe(link_section = ".sram1_bss")]
static TX_BUFFER: GroundedArrayCell<u32, DMA_BUFFER_LENGTH> = GroundedArrayCell::uninit();
#[unsafe(link_section = ".sram1_bss")]
static RX_BUFFER: GroundedArrayCell<u32, DMA_BUFFER_LENGTH> = GroundedArrayCell::uninit();

// - Interrupts ---------------------------------------------------------------
bind_interrupts!(pub struct AudioIrqs{
    DMA1_STREAM0 => dma::InterruptHandler<embassy_stm32::peripherals::DMA1_CH0>;
    DMA1_STREAM1 => dma::InterruptHandler<embassy_stm32::peripherals::DMA1_CH1>;
});

// - types --------------------------------------------------------------------

pub type InterleavedBlock = [u32; HALF_DMA_BUFFER_LENGTH];

/// `AudioPeripherals` is a builder to make `Interface` safely.
/// It ensures the correct pin mappings and DMA regions for
/// SAI on every supported Seed revision, preventing invalid peripheral
/// configurations at compile time.
/// Use `prepare_interface()` to apply board‐rev-specific SAI setup
/// and transition into the `Interface<'_, Idle>`. From there you can call `start_interface()` to move to
/// `Interface<'_, Running>` and begin audio callbacks.
pub struct AudioPeripherals<'a> {
    pub codec_pins: CodecPins<'a>,
    pub sai1: Peri<'a, hal::peripherals::SAI1>,
    pub i2c2: Peri<'a, hal::peripherals::I2C2>,
    pub dma1_ch0: Peri<'a, hal::peripherals::DMA1_CH0>,
    pub dma1_ch1: Peri<'a, hal::peripherals::DMA1_CH1>,
}

impl<'a> AudioPeripherals<'a> {
    /// Prepares the audio interface.
    ///
    /// This method sets up the SAI transmitter and receiver, configures the codec (if necessary),
    /// allocates DMA buffers, and applies board-specific SAI settings. It returns an `Interface<'a, Idle>`
    /// in the Idle state, allowing the runtime to decide when to start audio callbacks using `start_interface()`.
    ///
    /// # Arguments
    /// * `audio_config` - Audio configuration parameters such as the sample rate.  
    ///   You can use `AudioConfig::default()` or `Default::default()` for default settings.
    ///
    /// # Notes
    /// - This method is async because some codecs require configuration or
    ///   power-settling delays before their audio clocks start.
    /// - The board revision is selected via Cargo features.
    pub async fn prepare_interface(self, audio_config: AudioConfig) -> Interface<'a, Idle> {
        let tx_buffer: &mut [u32] = unsafe {
            TX_BUFFER.initialize_all_copied(0);
            let (ptr, len) = TX_BUFFER.get_ptr_len();
            core::slice::from_raw_parts_mut(ptr, len)
        };

        let rx_buffer: &mut [u32] = unsafe {
            RX_BUFFER.initialize_all_copied(0);
            let (ptr, len) = RX_BUFFER.get_ptr_len();
            core::slice::from_raw_parts_mut(ptr, len)
        };

        Interface {
            codec: Codec::new(self, audio_config, tx_buffer, rx_buffer).await,
            _state: PhantomData,
        }
    }
}

pub struct Idle {}
pub struct Running {}
pub trait InterfaceState {}
impl InterfaceState for Idle {}
impl InterfaceState for Running {}

/// decides when and how you start audio callback at runtime.
/// It enforces a two-state model:
///
/// * **Idle** – peripherals configured but SAI not started.
/// * **Running** – SAI started, ready to execute audio callbacks.
///
/// Transition from Idle to Running by calling `start_interface()`, which performs
/// codec register writes, waits for codec timing, and starts the SAI receiver and transmitter.
/// Once Running, invoke `start_callback()` to enter a continuous read→process→write loop. Any SAI errors are returned
/// to the caller for custom handling.
///
/// `Interface<'a, S>` manages the setup and runtime of an SAI-based audio stream.
/// It drives codec initialization (over I2C if required), configures SAI TX/RX,
/// and enforces a two-state model:
///
/// * **Idle** – peripherals configured but SAI not started.
/// * **Running** – SAI started, ready to execute audio callbacks.
///
/// Transition from Idle to Running by calling `start_interface()`, which performs
/// codec register writes, waits for codec timing, and starts the SAI receiver and transmitter.
/// Once Running, invoke `start_callback()` to enter a continuous read→process→write loop. Any SAI errors are returned
/// to the caller for custom handling.
///
/// # Example
/// ```rust
/// // 1. Configure peripherals into Idle state
/// let idle: Interface<Idle> = board
///     .audio_peripherals
///     .prepare_interface(Default::default())
///     .await;
///
/// // ... initialize your DSP or other resources ...
///
/// // 2. Start interface and transition to Running
/// let mut audio: Interface<Running> = idle
///     .start_interface()
///     .await
///     .unwrap();
///
/// // 3. Audio processing loop with error handling
/// loop {
///     // Runs until an SAI error occurs, then returns Err(e)
///     if let Err(e) = audio
///         .start_callback(|input, output| {
///             // process `input` samples into `output` buffer
///         })
///         .await
///     {
///         // handle SAI error e (be quick to avoid overrun)
///     }
///
///     // ... optionally reset or reinitialize DSP ...
/// }
/// ```
/// # Notes
/// - Always call `start_interface()` before `start_callback()`.
/// - Keep callback and error-handling routines short to prevent SAI overruns.
pub struct Interface<'a, S: InterfaceState> {
    codec: Codec<'a>,
    _state: PhantomData<S>,
}

impl<'a> Interface<'a, Idle> {
    /// This has to be called before `Interface::start_callback` can be used to ensure proper setup of the interface.
    /// `Interface::start_callback` should be called immediately afterwards otherwise overruns of the SAI can occur.
    pub async fn start_interface(mut self) -> Result<Interface<'a, Running>, sai::Error> {
        self.codec.start().await?;
        Ok(Interface {
            codec: self.codec,
            _state: PhantomData,
        })
    }
}

impl Interface<'_, Running> {
    pub async fn start_callback(
        &mut self,
        mut callback: impl FnMut(&[u32], &mut [u32]),
    ) -> Result<Infallible, sai::Error> {
        info!("enter audio callback loop");
        let mut write_buf = [0; HALF_DMA_BUFFER_LENGTH];
        let mut read_buf = [0; HALF_DMA_BUFFER_LENGTH];
        loop {
            self.codec.read(&mut read_buf).await?;
            callback(&read_buf, &mut write_buf);
            self.codec.write(&write_buf).await?;
        }
    }
}

impl<S: InterfaceState> Interface<'_, S> {
    pub fn sai_rx_config(&self) -> &sai::Config {
        &self.codec.sai_rx_config
    }

    pub fn sai_tx_config(&self) -> &sai::Config {
        &self.codec.sai_tx_config
    }
}
#[derive(Clone, Copy)]
pub enum Fs {
    Fs8000,
    Fs32000,
    Fs44100,
    Fs48000,
    Fs88200,
    Fs96000,
}
const CLOCK_RATIO: u32 = 256; //Not yet support oversampling.
impl Fs {
    pub fn into_clock_divider(self) -> MasterClockDivider {
        let fs = match self {
            Fs::Fs8000 => 8000,
            Fs::Fs32000 => 32000,
            Fs::Fs44100 => 44100,
            Fs::Fs48000 => 48000,
            Fs::Fs88200 => 88200,
            Fs::Fs96000 => 96000,
        };
        let kernel_clock = hal::rcc::frequency::<hal::peripherals::SAI1>().0;
        let mclk_div = (kernel_clock / (fs * CLOCK_RATIO)) as u8;
        mclk_div_from_u8(mclk_div)
    }
}

pub struct AudioConfig {
    pub fs: Fs,
}

impl Default for AudioConfig {
    fn default() -> Self {
        AudioConfig { fs: Fs::Fs48000 }
    }
}

//================================================

const fn mclk_div_from_u8(v: u8) -> MasterClockDivider {
    match v {
        1 => MasterClockDivider::DIV1,
        2 => MasterClockDivider::DIV2,
        3 => MasterClockDivider::DIV3,
        4 => MasterClockDivider::DIV4,
        5 => MasterClockDivider::DIV5,
        6 => MasterClockDivider::DIV6,
        7 => MasterClockDivider::DIV7,
        8 => MasterClockDivider::DIV8,
        9 => MasterClockDivider::DIV9,
        10 => MasterClockDivider::DIV10,
        11 => MasterClockDivider::DIV11,
        12 => MasterClockDivider::DIV12,
        13 => MasterClockDivider::DIV13,
        14 => MasterClockDivider::DIV14,
        15 => MasterClockDivider::DIV15,
        16 => MasterClockDivider::DIV16,
        17 => MasterClockDivider::DIV17,
        18 => MasterClockDivider::DIV18,
        19 => MasterClockDivider::DIV19,
        20 => MasterClockDivider::DIV20,
        21 => MasterClockDivider::DIV21,
        22 => MasterClockDivider::DIV22,
        23 => MasterClockDivider::DIV23,
        24 => MasterClockDivider::DIV24,
        25 => MasterClockDivider::DIV25,
        26 => MasterClockDivider::DIV26,
        27 => MasterClockDivider::DIV27,
        28 => MasterClockDivider::DIV28,
        29 => MasterClockDivider::DIV29,
        30 => MasterClockDivider::DIV30,
        31 => MasterClockDivider::DIV31,
        32 => MasterClockDivider::DIV32,
        33 => MasterClockDivider::DIV33,
        34 => MasterClockDivider::DIV34,
        35 => MasterClockDivider::DIV35,
        36 => MasterClockDivider::DIV36,
        37 => MasterClockDivider::DIV37,
        38 => MasterClockDivider::DIV38,
        39 => MasterClockDivider::DIV39,
        40 => MasterClockDivider::DIV40,
        41 => MasterClockDivider::DIV41,
        42 => MasterClockDivider::DIV42,
        43 => MasterClockDivider::DIV43,
        44 => MasterClockDivider::DIV44,
        45 => MasterClockDivider::DIV45,
        46 => MasterClockDivider::DIV46,
        47 => MasterClockDivider::DIV47,
        48 => MasterClockDivider::DIV48,
        49 => MasterClockDivider::DIV49,
        50 => MasterClockDivider::DIV50,
        51 => MasterClockDivider::DIV51,
        52 => MasterClockDivider::DIV52,
        53 => MasterClockDivider::DIV53,
        54 => MasterClockDivider::DIV54,
        55 => MasterClockDivider::DIV55,
        56 => MasterClockDivider::DIV56,
        57 => MasterClockDivider::DIV57,
        58 => MasterClockDivider::DIV58,
        59 => MasterClockDivider::DIV59,
        60 => MasterClockDivider::DIV60,
        61 => MasterClockDivider::DIV61,
        62 => MasterClockDivider::DIV62,
        63 => MasterClockDivider::DIV63,
        _ => panic!(),
    }
}
