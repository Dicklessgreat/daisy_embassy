# daisy-embassy

`daisy-embassy` is a Rust crate for building **embedded async audio applications** on the [Daisy Seed](https://electro-smith.com/products/daisy-seed) using the [Embassy framework](https://github.com/embassy-rs/embassy). It provides a streamlined interface to initialize and configure Daisy Seed hardware for **both** low-latency, non-blocking audio processing **and** powerful asynchronous application processing, making it an **ideal starting point** for embedded audio projects in Rust.

This crate is designed for developers familiar with embedded systems and audio processing, but new to Rust's embedded ecosystem. It enables safe and flexible audio application development, leveraging Rust's type system to prevent common peripheral configuration errors at compile time.

## Key Features

- **Audio Processing as an Integrated Task**: You can treat audio processing as a dedicated task within the Embassy async runtime, ensuring low-latency, non-blocking audio output. This approach allows seamless integration with other system tasks, enabling efficient resource sharing and predictable performance for audio applications on the Daisy Seed.
- **Asynchronous Application Processing**: Leverage the power of Embassy's async framework to build responsive and efficient applications. `daisy-embassy`(and embassy) supports `async` handling of GPIO interrupts, MIDI message stream reading/writing, OLED display updates, AD/DA streaming, and other application logic, allowing developers to create complex, event-driven audio projects with ease.
- **Simplified Setup**: Use the `new_daisy_board!` macro to initialize Daisy Seed peripherals with minimal boilerplate.
- **Safe Configuration**: Get sane clock defaults via `daisy_embassy::default_rcc`, and avoid the usual headaches of manual peripheral and DMA setup.
- **Flexible API**: Access peripherals through builder structs for safe defaults, or dive deeper with public accessors for custom configurations.
- **Community-Inspired**: Built on foundations from [stm32h7xx-hal](https://github.com/stm32-rs/stm32h7xx-hal), [daisy_bsp](https://github.com/antoinevg/daisy_bsp), [zlosynth/daisy](https://github.com/zlosynth/daisy), and [libdaisy-rust](https://github.com/mtthw-meyer/libdaisy-rust).

---

## Quick Start: Audio Passthrough Example

To demonstrate the ease of use, here's a simplified version of the `passthrough.rs` example, which sets up an audio passthrough (input to output) using the `new_daisy_board!` macro:

```rust
// safe clock configuration
let config = daisy_embassy::default_rcc();
// initialize the "board"
let p = hal::init(config);
let board: DaisyBoard<'_> = new_daisy_board!(p);

// build the "interface"
let mut interface = board
    .audio_peripherals
    .prepare_interface(Default::default())
    .await;

// start audio interface
let mut interface = unwrap!(interface.start_interface().await);
// start audio callback
unwrap!(
    interface
        .start_callback(|input, output| {
            // process audio data
            // here, we just copy input to output
            output.copy_from_slice(input);
        })
        .await
);
```

### How It Works

- **Macro Simplicity**: The `new_daisy_board!` macro moves necessary objects from `embassy_stm32::Peripherals` into builders like `daisy_embassy::AudioPeripherals` or `daisy_embassy::FlashBuilder` and so on, streamlining peripheral initialization.
- **Builder Pattern**: Peripherals are accessed via a `XXXBuilder` struct, which provides builder methods (in the case above, `.prepare_interface()`) for safe configuration.
- **Flexibility**: Builders expose `pub` accessors, allowing advanced users to bypass our building and implement custom initialization logic for peripherals.
- **Safety**: The API ensures memory safety and correct peripheral usage, aligning with Rust's guarantees.

See the `examples/` directory for more demos, such as `blinky.rs` or `triangle_wave_tx.rs`.

---

## Supported Daisy Boards

| Board               | Revision | Codec   | Status        | Tested |
|---------------------|----------|---------|---------------|-------------------------|
| Daisy Seed 1.1      | Rev5     | WM8731  | ✅ Supported  | 0.3.0(unreleased)  |
| Daisy Seed 1.2      | Rev7     | PCM3060 | ✅ Supported  | 0.2.2  |
| Daisy Seed (AK4556) | Rev4     | AK4556  | ✅ Supported  | 0.3.0(unreleased)  |
| Daisy Seed3         | Seed3    | TAC5242 | ✅ Supported  | 0.3.0(unreleased)  |
| Daisy Patch SM      | -        | PCM3060 | ✅ Supported  | 0.3.0(unreleased)  |

[Test reports and confirmations for `0.3.0(unreleased)` on Daisy Seed 1.2 are very welcome.](https://github.com/daisy-embassy/daisy-embassy/issues/59)

---

## Getting Started

### Prerequisites

- **Rust Toolchain**: Install via [rustup](https://rustup.rs/):

    ```bash
    rustup target add thumbv7em-none-eabihf
    ```

- **probe-rs**: For flashing and debugging, [install probe-rs](https://probe.rs/docs/getting-started/installation/).

- **Hardware**: Supported board (Daisy Seed Rev4, Rev5, or Rev7, or Daisy Patch SM), USB cable, and debug probe(e.g. ST-Link V3 MINIE).

> **Tip**: If probe-rs fails, verify your board connection and check [probe-rs docs](https://probe.rs/docs/overview/about-probe-rs/).

### Setup and Run

1. **Clone the Repository**:

   ```bash
   git clone https://github.com/daisy-embassy/daisy-embassy.git
   cd daisy-embassy
   ```

2. **Identify Your Board**:
   - Daisy Seed (AK4556): Use `--features=seed`.
   - Daisy Seed Rev5 (WM8731): `--features=seed_1_1`.
   - Daisy Seed Rev7 (PCM3060): Use `--features=seed_1_2`.
   - Daisy Seed3 (TAC5242): Use `--features=seed3`.
   - Daisy Patch SM: Use `--features=patch_sm`.

3. **Run an Example**:

   ```bash
   # Rev4(AK4556): Passthrough example
   cargo run --example passthrough --features=seed --release

   # Rev5(WM8731): Blinky example
   cargo run --example blinky --features=seed_1_1 --release

   # Rev7(PCM3060): Triangle wave example
   cargo run --example triangle_wave_tx --features=seed_1_2 --release

   # Seed3(TAC5242): Passthrough example
   cargo run --example passthrough --features=seed3 --release

   # Path SM: looper example
   cargo run --example looper --features=patch_sm --release
   ```

4. **Build and Customize**:
   - Explore `examples/` for demos like `passthrough.rs` or `triangle_wave_tx.rs`.
   - Modify examples to create custom audio applications.
   - Debug issues using probe-rs logs.
   - When you find a bug, need help, or have suggestions, open an [Issue](https://github.com/daisy-embassy/daisy-embassy/issues).

---

## Development Setup

### IDE Configuration

This crate uses feature flags to gate board-specific code. When developing in an IDE, you must configure rust-analyzer to enable the appropriate feature flag for your board, otherwise you may encounter compilation errors.

**For VS Code**: See `.vscode/settings.json` for an example configuration. Uncomment the feature flag corresponding to your board.

**For Other Editors**: Refer to the VS Code settings as a template and configure your editor accordingly to enable the appropriate feature flag:
- Daisy Seed (AK4556): `seed`
- Daisy Seed Rev5 (WM8731): `seed_1_1`
- Daisy Seed Rev7 (PCM3060): `seed_1_2`
- Daisy Seed3 (TAC5242): `seed3`
- Daisy Patch SM: `patch_sm`

---

## Sample Projects

[daisy-patch-embassy](https://github.com/daisy-embassy/daisy-patch-embassy)

---

## Announcements

- [add Seed Rev4(AK4556) support(version `0.2.2`)](https://github.com/daisy-embassy/daisy-embassy/discussions/57)
- [add Daisy Patch SM support(version `0.2.1`)](https://github.com/daisy-embassy/daisy-embassy/discussions/51)
- [version `0.2.0`(and `0.1.0`)](https://github.com/daisy-embassy/daisy-embassy/discussions/42)

---

## Resources

- [Daisy](https://daisy.audio/hardware/)
- [Embassy Documentation](https://github.com/embassy-rs/embassy)
- [probe-rs Guide](https://probe.rs/docs/overview/about-probe-rs/)
- [Daisy Community Forum](https://forum.electro-smith.com/) for hardware-related questions.

---

## License

This project is licensed under the [MIT License](LICENSE).
