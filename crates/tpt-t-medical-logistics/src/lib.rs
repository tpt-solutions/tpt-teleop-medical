//! Hospital delivery-robot logic for tpt-teleop-medical (spec.txt §5.2).
//!
//! * [`zones`] enforces infection control. A robot that has left a clean zone
//!   must complete a UV-C cycle before it can re-enter one.
//! * [`elevator`] gives each cab a single occupant, with emergency transports
//!   served first.
//! * [`route`] plans corridor routes. The infection-control rule is part of the
//!   search, so a forbidden route is never returned.
//! * [`profile`] sets what each load may pass through and where it may end:
//!   meals, medication and lab samples stay in clean space, and biohazard is
//!   never left in clean space.
//! * [`mqtt`] carries elevator requests as MQTT 3.1.1 `QoS 0` packets. The framing
//!   is written here, so no client dependency is added.
//!
//! The route planner does not model door timing or traffic between robots. The
//! MQTT framing does not open a broker connection. Both are left to the
//! deployment.

#![forbid(unsafe_code)]

pub mod elevator;
pub mod mqtt;
pub mod profile;
pub mod route;
pub mod zones;

pub use elevator::{Elevator, Grant, NotOccupant};
pub use mqtt::{ElevatorService, Message};
pub use profile::{Profile, ProfileError};
pub use route::{Map, Node, NodeId, Route, RouteError};
pub use zones::{Robot, Zone, ZoneError};
