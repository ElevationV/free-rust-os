#![allow(unused)]

#[cfg(feature="cortex-m3")]
mod cortex_m3;
#[cfg(feature="cortex-m3")]
pub use cortex_m3::*;
