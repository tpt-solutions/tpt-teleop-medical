//! Wait-free single-producer/single-consumer ring buffer for the haptic and
//! tracking handoff (spec.txt §4.2).
//!
//! The only public type is [`spsc::SpscRing`]. Its `unsafe` code is confined
//! to the `spsc` module; every other module in this crate forbids it.

#[allow(unsafe_code)]
pub mod spsc;

pub use spsc::SpscRing;
