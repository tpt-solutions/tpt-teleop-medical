//! Central event loop and surgical state machine for tpt-teleop-medical.
//!
//! The state machine is the single place where the surgical system decides
//! whether a command is allowed. Its states are
//! `Idle → Registered → Active → Paused`, with `Emergency` reachable from
//! every state.
//!
//! Producers push [`Event`]s into per-producer SPSC lanes. The [`EventLoop`]
//! drains those lanes on each tick and applies the events to the state
//! machine. Lane 0 is the emergency lane and is drained first.
//!
//! The state machine is pure logic: no allocation, no I/O, no clock. Only the
//! lanes allocate, and only at construction.
//!
//! [`audit::AuditedLoop`] couples the loop to the audit journal. Every event is
//! written to the journal before it takes effect, and a safety stop is
//! journalled and applied.

#![forbid(unsafe_code)]

pub mod audit;
pub mod event_loop;
pub mod state;

pub use audit::AuditedLoop;
pub use event_loop::{EMERGENCY_LANE, EventLoop, TickReport};
pub use state::{Event, InvalidTransition, Machine, State, transition};
