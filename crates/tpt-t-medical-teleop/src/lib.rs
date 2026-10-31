//! Telesurgery adapter for tpt-teleop (spec §4.6).
//!
//! [`adapter::Adapter`] implements `tpt-t-domain-bridge`'s
//! `DomainTeleopInterface` for the surgical arm. Wrap it in
//! `DomainSession` to get the handover lifecycle, which rejects out-of-order
//! calls before they reach the adapter.
//!
//! The adapter does not talk to the network. The caller reads commands from
//! tpt-teleop and calls [`adapter::Adapter::tick`] at 1 kHz.

#![forbid(unsafe_code)]

pub mod adapter;
pub mod link;

pub use adapter::{Actuators, Adapter, Config, JointReading};
