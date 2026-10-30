# Changelog

All notable changes to this project are documented in this file. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `tpt-t-medical-bridge-dicom`: bounded metadata parser for uncompressed Part 10 files (Explicit and Implicit VR Little Endian). It extracts the patient ID, study UID, modality and image size. Pixel data is skipped by length. Compressed syntaxes, undefined-length elements and file sizes over the limit are refused. Network services (C-FIND, C-MOVE, C-STORE) and SR export are not implemented.
- `tpt-t-medical-bridge-hl7fhir`: bounded HL7v2 ADT parser, dependency-free. It reads admit, transfer and discharge (A01, A02, A03) and keeps only the patient identifier and optional location. It does not read names or dates. Size limits cover message bytes, segments, fields and field length. Debug output redacts identifiers. A seeded mutation test checks that malformed input never panics. FHIR is not yet implemented, and it needs a JSON dependency that is awaiting approval.
- Phase 2 closing items in `tpt-t-medical-hipaa`: consent revocation (`RevocationLedger`, journalled and replayed on open; a revoked token is refused even with a valid signature), and an access audit (`ReleaseAudit`) that journals every attempt, granted or refused, before any plaintext is returned. If the journal write fails, nothing is decrypted.
- `tpt-t-medical-pharmacy` narcotic custody: `CustodyLedger` journals every receipt and transfer through the compliance journal. Only the current holder can transfer, and the authenticated person must be that holder. The ledger is rebuilt by replaying the journal, and a validly signed record that breaks the custody rules stops the open.
- Phase 5 logic: `tpt-t-medical-pharmacy` (FEFO dispensing, all-or-nothing, expired lots never issued; cold-chain excursion alerts past a grace period, with non-finite readings treated as out of range), `tpt-t-medical-sterilize` (UV-C dose planning from geometry, post-cycle coverage check; reflection not credited), and `tpt-t-medical-logistics` (clean/dirty/isolation rules that require a complete UV cycle before re-entering a clean zone; single-occupancy elevator with emergency priority).
- `tpt-t-medical-haptic`: 1 kHz tick loop with an impedance contact model against a plane. It reads the newest sample from the lock-free ring, and holds the last force on a missed deadline, a missing or stale sample, or an invalid sample. Force is never zeroed.
- `tpt-t-medical-surgical`: motion scaling (1:1 to 10:1), RBJ band-stop tremor notch at 10 Hz with DF-II-T state, trilinear SDF lookup that returns unknown outside the grid, and a linear progressive fixture force. NaN distance gives maximum force.
- `tpt-t-medical-hipaa`: PHI encryption with AES-256-GCM. Each record gets a random 96-bit nonce and binds the patient identifier as associated data. Consent tokens are signed, purpose-limited and expiring. `release` checks consent before decrypting. `Phi` redacts its Debug output.
- `AuditedLoop` in `tpt-t-medical-core`: every transition is journalled before it is applied. Safety stops are journalled as a reason record followed by the transition, and they take effect even if the journal write fails. Event and state codes are fixed for the audit format.
- `tpt-t-medical-safety`: safety monitor. Workspace envelope, force limit (NaN refused), watchdog (gaps and clock regressions), latched emergency stop from four sources, and a TMR voter that requires all three copies to agree. Stops latch until an explicit reset. Timing targets are not verified.
- `tpt-t-medical-compliance`: append-only hash-chained audit log with SHA-256 digests over sequence, previous hash and length-prefixed payload. Verification reports the first broken entry and the check that failed. Ed25519 signatures come from `ed25519-dalek` (BSD-3-Clause) with `verify_strict`. The `ring` crate is not used because it is Apache-2.0 AND ISC.
- `tpt-t-medical-compliance`: journal that persists each entry before it is committed. File-backed WAL with a length-prefixed framing and sync after each append. Opening refuses a torn tail rather than repairing it, and a bad record or broken chain also stops the open.
- `tpt-t-medical-core`: surgical state machine (Idle, Registered, Active, Paused, Emergency). Emergency is latched and clears only to Idle. Pure logic with no allocation. The EventLoop drains one SPSC lane per producer on each tick. Lane 0 is the emergency lane and is drained first.
- CI: loom model-check job for `tpt-t-medical-ring`.
- `tpt-t-medical-ring`: wait-free SPSC ring buffer (SpscRing), adapted from the
  tpt-teleop `tpt-t-ring` crate. Unused modules (shared memory, pointer rings,
  byte casting, layout) are left out. Unsafe code is confined to one module.
  Verified with unit tests and loom model checks, including a deliberately
  weakened ordering that the loom tests catch.
- Workspace scaffolding: `Cargo.toml` (resolver 3, edition 2024, MSRV 1.85,
  workspace version 1.0.0), `rust-toolchain.toml`, and `deny.toml`.
- Dual `MIT OR Apache-2.0` license files.
- CI workflows for build and test on Linux and Windows (stable and 1.85), and
  for rustfmt, clippy and cargo-deny.
- `README.md`, `SECURITY.md`, `docs/ARCHITECTURE.md`, and regulatory
  scaffolding templates under `docs/regulatory/`.
