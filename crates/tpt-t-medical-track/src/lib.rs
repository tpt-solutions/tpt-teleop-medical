//! Tool and anatomy tracking for tpt-teleop-medical (spec.txt §4.4).
//!
//! * [`filter::Filter`] fuses position reports from several sensors, such as
//!   optical and electromagnetic, with a constant-velocity Kalman filter.
//!   Reports far from the prediction are rejected.
//! * [`register::register`] finds the rigid transform from the tracker's frame
//!   to the pre-op scan's frame, from paired fiducials.
//!
//! The measurement models here are linear, so the filter is the extended
//! filter with an identity Jacobian. Nonlinear sensor models, and the portable
//! SIMD the spec asks for, are not here. Latency and the sub-millimetre target
//! are checked on simulated data only. They need the target sensors.

#![forbid(unsafe_code)]

pub mod filter;
pub mod register;

pub use filter::{Filter, Outcome};
pub use register::{Rigid, register};
