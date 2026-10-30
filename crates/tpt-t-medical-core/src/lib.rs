//! Central event loop and surgical state machine for tpt-teleop-medical.
//!
//! The state machine is the single place where the surgical system decides
//! whether a command is allowed. Its states are
//! `Idle → Registered → Active → Paused`, with `Emergency` reachable from
//! every state.
//!
//! The machine is pure logic: no allocation, no I/O, no clock. It can run on
//! the real-time core and under the simulator alike.

#![forbid(unsafe_code)]

pub mod state;

pub use state::{Event, InvalidTransition, Machine, State, transition};
