# Changelog

All notable changes to this project are documented in this file. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Known issues

- `cargo deny check bans` fails on two wildcard path dependencies in the tpt-teleop repository: `tpt-t-core` takes `tpt-t-ring`, and `tpt-t-domain-bridge` takes `tpt-t-core`, with `workspace = true` and no version. Adding `version` to those entries in tpt-teleop's workspace clears it. Licences and sources pass.

### Added

- `tpt-t-medical-bridge-dicom`: CT pixel ingestion in `pixels`. `read_slice` reads one uncompressed 16-bit MONOCHROME2 slice to Hounsfield units. `read_volume` orders a series by `ImagePositionPatient` and refuses non-uniform spacing, mismatched sizes and volumes above 64 M voxels.
- `tpt-t-medical-ai-session`: `Fleet::pending` (a FIFO queue of units awaiting a surgeon, derived from unit state) and a read-only `Fleet::summary`. The dispatcher still has no authority beyond requesting.
- `tpt-t-medical-teleop`: `link` module, a fixed-size link-quality monitor. Nominal within 10 ms, degraded up to the 50 ms horizon, lost beyond it or on a clock fault. `Adapter::observe_link` latches the adapter on a lost link, and a silent link past the horizon also latches once a transport has reported.
- `tpt-t-medical-sim`: `haptic::HapticRig`, which renders contact force from the simulated tool using the impedance model of `tpt-t-medical-haptic`.
- `tpt-t-medical-teleop`: two-site session handling. `Adapter::observe_feed_link` adds the patient-to-surgeon path. `link_state` is the worse of the two paths, and a lost feed latches the arm as a lost command path does. `tests/two_site.rs` drives both paths over the simulator's delay link.
- `tpt-t-medical-vision`: `segmentation`, which turns a reviewed mask into the signed-distance fixture grid in metres. Anisotropic input is resampled to the finest spacing, then passed to `volume::signed_distance`. `window` thresholds Hounsfield units as a helper.
- `tpt-t-medical-visualizer`: `html::render_live`, a self-refreshing operator page. It replaces the planned egui window, whose dependency tree includes Apache-2.0-only crates that `deny.toml` rejects.
- `tpt-t-medical-visualizer`: `feed::Feed`, a bounded live feed. The running system pushes a frame per tick and audit events, and the feed builds the scene from them. It is driven end to end from the simulator in `tests/live.rs`. No window toolkit is used.
- Property and budget suites for the notch filter, SDF lookup, HL7 and DICOM parsers, and the TMR voter. An end-to-end telesurgery flow test runs spec §6 across the adapter, safety, audit and predictive display.

- `tpt-t-medical-teleop` (new crate): the surgical arm as a `DomainTeleopInterface` adapter from the tpt-teleop domain bridge, linked by path to `../tpt-teleop/crates/tpt-t-domain-bridge`. Commands pass through a 10 Hz tremor notch, motion scaling, and the virtual-fixture guard before they move a tool target. The tool is tracked by a spring and damper through the edge torque pipeline. The safety monitor runs on every tick, and a stop zeroes the torque and latches. Engage starts at the current tool position, and disengage ramps the tracking force away while gravity compensation holds the arm. Tests drive the adapter through `DomainSession`.
- `tpt-t-medical-vision` (new crate): block-matching stereo with a left-right check, depth from disparity, back-projection to a point cloud, AR overlay of scan points through the registration, and segmentation to a signed-distance grid for the virtual fixtures, using an exact separable distance transform. Checked on synthetic images.
- `tpt-t-medical-bridge-dicom`: DICOM network client over TCP (`net` module), with Implicit and Explicit VR encoding, PDU framing, and the C-ECHO, C-FIND, C-GET and C-STORE operations. C-GET is used for retrieval, since C-MOVE needs an inbound listener. Tests run the client against a loopback SCP. `sr` exports a Basic Text Structured Report as a Part 10 file, and that export has not been conformance-validated.
- `tpt-t-medical-mcp` (new crate): MCP supervisory interface over newline-delimited JSON-RPC 2.0. It exposes `list_units`, `unit_status` and `audit_summary` (read-only), and `request_assistance` (one journalled write, sent as the configured dispatcher). It has no reset, release, limit or veto tool, and its inputs are validated strictly.
- `tpt-t-medical-visualizer` (new crate): the operator scene model, with trajectory, workspace box, fixture boundary, force arrows, audit timeline, TMR state, and a view projection. It writes a self-contained HTML page (`html::render`, example `export`) with inline SVG and a small script that rotates the view by dragging. The page has no external resources, and its projection matches the Rust model.
- `tpt-t-medical-ai-session` (new crate): the AI controller as an assist-only operator, following the recommendations in `docs/design/ai-session.md`. The AI can only propose, and each proposal is checked for envelope, force, a contradiction with the operator's motion, and rate. Only the operating surgeon can engage, release, override, renew or reset. Sessions expire, a missed heartbeat is a fault, and a session opens latched. Every command, refusal and state change is journalled through `tpt-t-medical-compliance`. A fleet exposes only assistance requests to dispatchers, with one journal per unit. AI reads of PHI go through the HIPAA consent gate, and logging is refused.
- `tpt-t-medical-edge` (new crate): 1 kHz torque pipeline. It maps tool force to joint torque through the Jacobian, adds gravity compensation and friction, clamps to the torque limits, and holds the last command on a missed deadline. The virtual-work identities are checked. The real-time core is not.
- `tpt-t-medical-track` (new crate): constant-velocity Kalman filter fusing optical and electromagnetic position, with outlier gating, and Horn rigid registration to a pre-op scan. Checked on simulated data only.
- `tpt-t-medical-bridge-hl7fhir`: `pharmacy::dispense_order` dispenses an `active` `MedicationRequest` from pharmacy stock, all or nothing. `logistics::transport_for` turns an admit or transfer into a transport destination, and a discharge into a clear. Unknown locations are refused.
- `tpt-t-medical-pharmacy`: unit-dose packaging. Each dose is checked by barcode and by weight, within 5 %, and the package seals when complete.
- `tpt-t-medical-sterilize`: LiDAR room mapping. Returns are binned into a voxel grid, and lit faces are found with shadowing. The faces feed the existing dose planner.
- `tpt-t-medical-logistics`: UV-aware route planning. A contaminated robot must pass a UV station before it re-enters clean space, and the search enforces that. Transport profiles restrict what each load may enter. MQTT 3.1.1 QoS 0 framing is written in-house, with an elevator request/grant/release service, so no client dependency is added.
- `tpt-t-medical-surgical`: 7-DOF forward kinematics with a reference DH table, a predictive display capped at a 50 ms horizon, and a virtual-fixture guard that blocks motion into a structure and overrides the master on persistent intrusion.
- `tpt-t-medical-sim`: deterministic command latency and jitter, a mass-spring tissue, tool position from joint readings, and an integration test of five AI fault scenarios against the AI session.
- `tpt-t-medical-compliance`: `direct::DirectWal`, a block-aligned log. Records are padded to 4 KiB, and the reader rejects non-zero padding. On Windows it opens with `FILE_FLAG_NO_BUFFERING` and `FILE_FLAG_WRITE_THROUGH`. Linux `O_DIRECT` is not set.
- `tpt-t-medical-safety`: `Limits::watchdog_timeout_us`, so a session can check its heartbeat against the watchdog.
- `tpt-t-medical-bridge-hl7fhir` audit posting: `BridgeAudit` journals each ADT event and each medication order through the compliance journal. Records hold the kind and opaque identifiers, not clinical content.
- `tpt-t-medical-bridge-hl7fhir` pharmacy feed: `pharmacy::dispense_order` dispenses an `active` `MedicationRequest` from `tpt-t-medical-pharmacy` stock (FEFO, all or nothing). Refuses non-active orders, missing, fractional or non-positive quantities, and short stock, leaving the stock unchanged.
- Documentation: README status and crate table refreshed to match the implemented crates.
- `tpt-t-medical-bridge-hl7fhir` FHIR: parses R4 `MedicationRequest` JSON (order ID, status, patient reference, drug, dispense quantity). Uses `serde_json` 1.x (MIT OR Apache-2.0), approved for this use. Limits: 256 KiB resource, 256-byte strings, a positive finite quantity, and a known status. `to_value` writes the order back. Debug redacts the patient. Other resource types are refused.
- `tpt-t-medical-sim`: headless simulator core for a seven-joint arm at a fixed 1 kHz step. Each joint has gravity, viscous and Coulomb friction, and a backlash operator on its reported position. Faults can be injected and cleared: sensor failure, motor stall, and network drop. Non-finite commands are refused. Runs are deterministic. Kinematics, tissue, haptics and the visualizer are not yet modelled.
- CI: a Miri job runs the ring crate's single-threaded unit tests under nightly miri.
- `tpt-t-medical-conformance`: cross-bridge tests, with no product code. They check that an admit precedes a transfer or discharge, that a CT scan matches only an admitted patient, that every ADT event is journalled and the journal verifies after restart, that malformed input from either bridge never panics, and that Debug output hides identifiers. The admission rules are a reference implementation for the tests.
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

### Fixed

- `tpt-t-medical-bridge-dicom`: the Basic Text SR export wrote its concept name and content sequences after the other top-level elements, which broke the ascending tag order that PS3.5 §7.1 requires. It also omitted Series Number and the Type 2 patient and study attributes. The export now sorts every top-level element by tag and includes them, and `tests/sr_conformance.rs` checks the result.

- `tpt-t-medical-teleop`: the virtual-fixture guard checked only the measured tool, so a lagging arm let the commanded target enter a structure. A motion is now applied only if the target's next position is clear. Otherwise it is blocked with full feedback.
