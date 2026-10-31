# tpt-t-medical-pharmacy

Automated pharmacy logic for tpt-teleop-medical (spec.txt §5.3, §5.2): stock
selection, unit-dose packaging with verification, cold-chain alerting and
narcotic chain-of-custody.

## Status

Logic done for FEFO dispensing, cold-chain alerting, narcotic custody and
barcode/weight packaging. There is no hardware integration for scales, scanners
or refrigerators.

## Overview

| Module | Main types | Purpose |
|---|---|---|
| `inventory` | `Inventory`, `Lot`, `Pick`, `DispenseError` | First-expired, first-out (FEFO) selection. Expired lots are never issued. Dispensing is all or nothing: a short-stock request leaves the stock unchanged. |
| `packaging` | `Package`, `Expected`, `Progress`, `VerifyError`, `DEFAULT_WEIGHT_TOLERANCE` | Verifies each unit dose by barcode and by weight, within 5 %. The package seals once every dose has passed. |
| `coldchain` | `ColdChain`, `Alert` | Tracks refrigerated storage. An alert is raised once an excursion lasts longer than a grace period. Non-finite readings count as out of range. |
| `custody` | `CustodyLedger`, `Holding`, `Authentication`, `Method`, `CustodyError` | Records each receipt and transfer of narcotics. Only the current holder can transfer, and the authenticated person must be that holder. `Method` is `Badge` or `Biometric`. |

`Progress` is `Open { remaining }` while doses are still needed, and
`Sealed { units }` once the package is complete.

### Custody and the journal

Every custody change is journalled through
[`tpt-t-medical-compliance`](../tpt-t-medical-compliance). The ledger is rebuilt
by replaying the journal on open. A validly signed record that breaks the custody
rules stops the open.

## Testing

```sh
cargo test -p tpt-t-medical-pharmacy
```

## Limitations

- The caller authenticates the person before `custody` is called. The ledger
  checks who the holder is, not the credential itself.
- Barcode and weight inputs are supplied by the caller. No device driver is here.

## Dependencies

- [`tpt-t-medical-compliance`](../tpt-t-medical-compliance)

## Related crates

- [`tpt-t-medical-bridge-hl7fhir`](../tpt-t-medical-bridge-hl7fhir) turns an
  active FHIR `MedicationRequest` into a dispense from this crate's stock.
- [`tpt-t-medical-logistics`](../tpt-t-medical-logistics) moves medication to the
  wards.

## Licence

MIT OR Apache-2.0. See the [workspace README](../../README.md).
