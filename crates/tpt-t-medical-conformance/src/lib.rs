//! Cross-bridge conformance for tpt-teleop-medical (Phase 6).
//!
//! This crate has no product code. Its checks live in `tests/conformance.rs`,
//! where the HL7 ADT parser, the DICOM metadata parser, and the compliance
//! journal are used together. Each check is a rule the hospital integration
//! depends on:
//!
//! * An admit must come before a transfer or discharge for the same patient.
//! * A CT scan may be matched only to a patient who is currently admitted.
//! * Every ADT event is journalled, and the journal verifies after a restart.
//! * Malformed input from either bridge is refused without a panic.
//! * Debug output from both bridges does not show identifiers.
//!
//! The admission rules are a reference implementation written for these tests.
//! They are not a product module.
