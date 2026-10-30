//! Hospital delivery-robot logic for tpt-teleop-medical (spec.txt §5.2).
//!
//! * [`zones`] enforces infection control. A robot that has left a clean zone
//!   must complete a UV-C cycle before it can re-enter one.
//! * [`elevator`] gives each cab a single occupant, with emergency transports
//!   served first.
//!
//! Path planning through corridors and doors, and the MQTT transport, are not
//! implemented here.

#![forbid(unsafe_code)]

pub mod elevator;
pub mod zones;

pub use elevator::{Elevator, Grant, NotOccupant};
pub use zones::{Robot, Zone, ZoneError};
