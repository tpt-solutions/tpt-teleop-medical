# Changelog

All notable changes to `tpt-t-medical-bridge-hl7fhir` are documented in this file.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the crate uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Bounded HL7 v2 ADT parser, dependency-free. It reads admit, transfer and
  discharge (A01, A02, A03) and keeps only the patient identifier and optional
  location. It does not read names or dates. Size limits cover message bytes,
  segments, fields and field length. Debug output redacts identifiers. A seeded
  mutation test checks that malformed input never panics.
- FHIR R4 `MedicationRequest` parsing (order ID, status, patient reference, drug,
  dispense quantity), using `serde_json` 1.x. Limits: 256 KiB resource, 256-byte
  strings, a positive finite quantity, and a known status. `to_value` writes the
  order back. Debug redacts the patient. Other resource types are refused.
- `BridgeAudit` journals each ADT event and each medication order through the
  compliance journal. Records hold the kind and opaque identifiers, not clinical
  content.
- `pharmacy::dispense_order` dispenses an `active` `MedicationRequest` from
  `tpt-t-medical-pharmacy` stock, all or nothing. It refuses non-active orders,
  missing, fractional or non-positive quantities, and short stock, leaving the
  stock unchanged.
- `logistics::transport_for` turns an admit or transfer into a transport
  destination, and a discharge into a clear. Unknown locations are refused.

### Known issues

- Only `MedicationRequest` is parsed from FHIR.
- HL7 escape sequences are not decoded. An identifier that contains one is refused.
