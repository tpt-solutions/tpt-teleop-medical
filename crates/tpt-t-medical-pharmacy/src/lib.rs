//! Automated pharmacy logic for tpt-teleop-medical (spec.txt §5.3).
//!
//! * [`Inventory`] chooses which lots to dispense. It uses first-expired,
//!   first-out (FEFO) ordering and never hands out an expired lot.
//! * [`packaging::Package`] verifies each unit dose by barcode and weight, and
//!   seals the package once every dose has passed.
//! * [`ColdChain`] tracks refrigerated storage and raises an alert once a
//!   temperature excursion lasts longer than a grace period.
//!
//! * [`custody::CustodyLedger`] records narcotic chain-of-custody (spec.txt
//!   §5.2). Each change of holder is signed and journalled through
//!   `tpt-t-medical-compliance`, and the ledger is rebuilt by replaying it.

#![forbid(unsafe_code)]

pub mod coldchain;
pub mod custody;
pub mod inventory;
pub mod packaging;

pub use coldchain::{Alert, ColdChain};
pub use custody::{Authentication, CustodyError, CustodyLedger, Holding, Method};
pub use inventory::{DispenseError, Inventory, Lot, Pick};
pub use packaging::{Expected, Package, Progress, VerifyError};
