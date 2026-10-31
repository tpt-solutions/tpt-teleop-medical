# Changelog

All notable changes to `tpt-t-medical-pharmacy` are documented in this file. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
crate uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- FEFO dispensing, all or nothing. Expired lots are never issued.
- Cold-chain excursion alerts past a grace period. Non-finite readings are treated
  as out of range.
- Narcotic custody: `CustodyLedger` journals every receipt and transfer through the
  compliance journal. Only the current holder can transfer, and the authenticated
  person must be that holder. The ledger is rebuilt by replaying the journal, and
  a validly signed record that breaks the custody rules stops the open.
- Unit-dose packaging. Each dose is checked by barcode and by weight, within 5 %,
  and the package seals when complete.
- Dispensing of active medication orders from pharmacy stock, through the HL7 and
  FHIR bridge.
