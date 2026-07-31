#[cfg(feature = "seed")]
mod ak4556;
#[cfg(feature = "seed")]
pub use ak4556::{Codec, Pins};

#[cfg(feature = "seed_1_1")]
mod wm8731;
#[cfg(feature = "seed_1_1")]
pub use wm8731::{Codec, Pins};

#[cfg(feature = "seed_1_2")]
mod pcm3060;
#[cfg(feature = "seed_1_2")]
pub use pcm3060::{Codec, Pins};

#[cfg(feature = "seed3")]
mod tac5242;
#[cfg(feature = "seed3")]
pub use tac5242::{Codec, Pins};

#[cfg(feature = "patch_sm")]
mod pcm3060_i2c;
#[cfg(feature = "patch_sm")]
pub use pcm3060_i2c::{Codec, Pins};
