//! Automated pharmacy logic for tpt-teleop-medical (spec.txt §5.3).
//!
//! * [`Inventory`] chooses which lots to dispense. It uses first-expired,
//!   first-out (FEFO) ordering and never hands out an expired lot.
//! * [`ColdChain`] tracks refrigerated storage and raises an alert once a
//!   temperature excursion lasts longer than a grace period.
//!
//! Narcotic chain-of-custody (spec.txt §5.2) is not implemented here. Its
//! records belong in `tpt-t-medical-compliance`, and the wiring is a later
//! step.

#![forbid(unsafe_code)]

pub mod coldchain;
pub mod inventory;

pub use coldchain::{Alert, ColdChain};
pub use inventory::{DispenseError, Inventory, Lot, Pick};
