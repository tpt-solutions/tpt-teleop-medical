# tpt-t-medical-bridge-hl7fhir

The HL7 and FHIR bridge for tpt-teleop-medical (Phase 6). It parses HL7 v2 ADT
messages and FHIR R4 `MedicationRequest` resources, journals them, and feeds them
to the pharmacy and logistics crates.

## Status

Both parsers, FHIR output, audit posting, and the pharmacy and transport feeds are
done. Other FHIR resource types are not implemented.

## Overview

| Module | Main items | Purpose |
|---|---|---|
| root | `AdtKind`, `AdtEvent`, `parse`, `ParseError`, `MAX_MESSAGE_BYTES`, `MAX_SEGMENTS`, `MAX_FIELDS`, `MAX_FIELD_BYTES` | The HL7 v2 ADT parser |
| `fhir` | `parse_medication_request`, `to_value`, `FhirError`, `MAX_RESOURCE_BYTES`, `MAX_STRING_BYTES` | The FHIR R4 `MedicationRequest` parser and writer |
| `audit` | `BridgeAudit<S, G>`, `record_adt`, `record_order`, `open` | Journals each ADT event and medication order through the compliance journal |
| `pharmacy` | `dispense_order`, `Refusal` | Dispenses an active order from pharmacy stock |
| `logistics` | `transport_for`, `Transport`, `Wards`, `LocationError` | Turns an admit or transfer into a transport destination, and a discharge into a clear |

### HL7 v2 ADT

Accepts admit (`ADT^A01`), transfer (`ADT^A02`) and discharge (`ADT^A03`). It reads
only:

- `MSH`: the message type (MSH-9) and encoding characters (MSH-2)
- `PID`: the patient identifier (PID-3, first component)
- `PV1`: the patient location (PV1-3, first component). Optional.

Names, addresses and dates of birth are not read. The parser depends on nothing
outside `std`. Escape sequences (`\F\`, `\S\` and similar) are not decoded, and an
identifier that contains one is refused.

### FHIR R4 MedicationRequest

Reads the order ID, status, patient reference, drug and dispense quantity.

- Resources over 256 KiB are refused.
- Strings over 256 bytes are refused.
- The quantity must be positive and finite.
- The status must be a known value.
- Other resource types are refused.

`to_value` writes an order back as JSON. Debug output redacts the patient.

### Pharmacy feed

`pharmacy::dispense_order` dispenses an `active` order from
[`tpt-t-medical-pharmacy`](../tpt-t-medical-pharmacy) stock. It is all or nothing.
It refuses non-active orders, missing, fractional or non-positive quantities, and
short stock. A refused order leaves the stock unchanged.

### Logistics feed

`logistics::transport_for` maps an admit or transfer to a destination in
[`tpt-t-medical-logistics`](../tpt-t-medical-logistics), and a discharge to a
clear. Unknown locations are refused.

### Audit

`BridgeAudit` journals each ADT event and each medication order. Records hold the
kind and opaque identifiers only, not clinical content.

## Testing

```sh
cargo test -p tpt-t-medical-bridge-hl7fhir
```

The HL7 parser has a seeded mutation test that checks malformed input never
panics.

## Limitations

- Only ADT A01, A02 and A03 are accepted.
- Only `MedicationRequest` is parsed from FHIR.
- No network listener. Messages are passed in by the caller.

## Dependencies

- `serde_json` 1 (FHIR JSON)
- [`tpt-t-medical-compliance`](../tpt-t-medical-compliance) (audit journal)
- [`tpt-t-medical-logistics`](../tpt-t-medical-logistics) (transport feed)
- [`tpt-t-medical-pharmacy`](../tpt-t-medical-pharmacy) (dispensing feed)

## Related crates

- [`tpt-t-medical-bridge-dicom`](../tpt-t-medical-bridge-dicom) is the imaging
  bridge.
- [`tpt-t-medical-conformance`](../tpt-t-medical-conformance) tests this bridge
  with the DICOM bridge and the audit journal.

## Licence

MIT OR Apache-2.0. See the [workspace README](../../README.md).
