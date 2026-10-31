//! Surgical signal and geometry processing for tpt-teleop-medical (spec.txt
//! §4.3, §4.5).
//!
//! * [`MotionScaler`] divides the surgeon's hand motion down to the tool's
//!   motion, from 1:1 for microsurgery to 10:1 for gross dissection.
//! * [`NotchFilter`] removes the 8–12 Hz physiological tremor band from the
//!   master input.
//! * [`SdfGrid`] looks up the signed distance to a critical structure, using
//!   trilinear interpolation.
//! * [`kinematics::Arm`] computes the tool pose from the seven joint angles.
//! * [`predict::Predictor`] is the predictive display: it shows the tool where
//!   it will be, and hides network delay of up to 50 ms.
//! * [`guard::Guard`] keeps the tool out of a forbidden structure, and overrides
//!   the master if the tool stays inside too long.
//! * [`fixture::repulsive_force`] turns that distance into the progressive force
//!   the surgeon feels as they approach a forbidden zone.
//!
//! Everything here is plain arithmetic on `f64`. The spec's timing targets
//! (<5 µs for the filters, <20 µs for the SDF) are not verified on this target.
//! They need the real-time hardware.

#![forbid(unsafe_code)]

pub mod fixture;
pub mod guard;
pub mod kinematics;
pub mod notch;
pub mod predict;
pub mod scaling;
pub mod sdf;

pub use guard::Guard;
pub use kinematics::Arm;
pub use notch::NotchFilter;
pub use predict::Predictor;
pub use scaling::MotionScaler;
pub use sdf::SdfGrid;
