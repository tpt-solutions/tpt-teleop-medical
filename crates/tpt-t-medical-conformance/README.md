# tpt-t-medical-conformance

Cross-bridge conformance tests for tpt-teleop-medical (Phase 6). The crate has no
product code. Its checks live in `tests/conformance.rs`, where the HL7 ADT parser,
the DICOM metadata parser and the compliance journal are used together.

## Status

Done. The tests run as part of the workspace test suite.

## What is checked

Each check is a rule that hospital integration depends on:

1. An admit must come before a transfer or discharge for the same patient.
2. A CT scan may be matched only to a patient who is currently admitted.
3. Every ADT event is journalled, and the journal verifies after a restart.
4. Malformed input from either bridge is refused without a panic.
5. Debug output from both bridges does not show identifiers.

The admission rules are a reference implementation written for these tests. They
are not a product module.

## Testing

```sh
cargo test -p tpt-t-medical-conformance
```

## Dependencies

Development-only:

- [`tpt-t-medical-bridge-dicom`](../tpt-t-medical-bridge-dicom)
- [`tpt-t-medical-bridge-hl7fhir`](../tpt-t-medical-bridge-hl7fhir)
- [`tpt-t-medical-compliance`](../tpt-t-medical-compliance)

## Licence

MIT OR Apache-2.0. See the [workspace README](../../README.md).
