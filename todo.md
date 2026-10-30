# tpt-teleop-medical — Project Checklist

Safety-critical, zero-bloat Rust middleware for telesurgery, hospital
logistics, automated pharmacy, and sterilization robotics. Dual-licensed
MIT OR Apache-2.0, copyright TPT Solutions. Crates use the house
`tpt-t-medical-*` prefix (spec.txt §3 already specifies this simplified
form directly — the repo itself keeps the full `tpt-teleop-medical` name).
See [spec.txt](spec.txt) and [bridge spec.txt](bridge%20spec.txt) for full
design detail.

Phases are ordered dependency-first: foundations → safety/compliance
backbone → surgical control → telesurgery networking → hospital
logistics → hospital-IT bridges → tooling/simulation → regulatory
scaffolding → hardening/release. Safety and audit come immediately after
foundations (not near the end) because every other crate logs through
compliance and is bounded by safety — see spec.txt §1/§4.

## Phase 0 — Project Scaffolding

- [x] Workspace `Cargo.toml` (resolver = "3", edition = "2024", rust-version = "1.85", workspace version 1.0.0)
- [x] Dual license files: `LICENSE-MIT`, `LICENSE-APACHE` (copyright TPT Solutions); `license = "MIT OR Apache-2.0"` in workspace manifest
- [x] `deny.toml` (cargo-deny) — allow MIT/BSD-2/BSD-3/ISC/Zlib/MPL-2.0; dual-licensed crates (e.g. `ring`, `rkyv`) resolved strictly under MIT; ban any strictly-Apache-2.0-only crate (spec.txt §2, §7)
- [x] `rust-toolchain.toml` (channel = stable, components = rustfmt, clippy)
- [x] `.github/workflows/ci.yml` (build + test matrix)
- [x] `.github/workflows/lint.yml` (fmt, clippy, cargo-deny check)
- [x] `README.md`, `CHANGELOG.md`, `SECURITY.md`
- [x] `docs/ARCHITECTURE.md` skeleton (workspace map, crate responsibilities, data-flow diagram from spec.txt §6)
- [x] `git init` + initial commit

## Phase 1 — Foundation Primitives

- [x] Decide lock-free SPSC/MPSC ring-buffer sourcing: reuse/vendor `tpt-teleop`'s ring pattern vs. new `tpt-t-medical-ring` (needed by the haptic↔vision/tracking handoff in spec.txt §4.2)
- [ ] `tpt-t-medical-core` — central event loop, surgical state machine (Idle → Registered → Active → Paused → Emergency), lock-free message bus

## Phase 2 — Safety & Compliance Backbone

- [ ] `tpt-t-medical-safety` — Triple-Modular Redundancy (TMR) voter across 3 isolated cores, zero-allocation static memory, independent watchdogs, force/torque limiters, workspace envelope enforcement, emergency-stop hierarchies (foot pedal, hand switch, voice, auto), <100us disagreement-to-safe-state
- [ ] `tpt-t-medical-compliance` — append-only hash-chained WAL; events serialized via rkyv, signed Ed25519 (TPM/secure-enclave-backed key), SHA-256 chained, written O_DIRECT to NVMe; tamper-evident (broken chain invalidates log); log export/seal for post-op regulatory archive
- [ ] `tpt-t-medical-hipaa` — PHI encryption at rest (AES-256-GCM) and in transit; consent-token gating so PHI never enters logs/training data without explicit consent

## Phase 3 — Surgical Control & Haptics

- [ ] `tpt-t-medical-edge` — 1kHz real-time control loop on dedicated RT core, Cartesian → joint-space torque translation, gravity compensation, friction modeling
- [ ] `tpt-t-medical-haptic` — 1kHz haptic rendering, pinned core + SCHED_FIFO, custom impedance control (stiffness/damping/tool-tissue forces), <50us loop / 1ms hard deadline, hold-last-value on missed deadline, lock-free SPSC ring buffer input
- [ ] `tpt-t-medical-track` — tool/anatomy tracking fusion (optical NDI-Polaris-style + electromagnetic + intra-op imaging), custom EKF (portable-simd), sub-millimeter registration to pre-op scan (<0.5mm)
- [ ] `tpt-t-medical-vision` — stereoscopic endoscope pipeline, 3D reconstruction, depth estimation, pre-op CT/MRI overlay onto live video (AR guidance), critical-structure segmentation ingestion for virtual fixtures
- [ ] `tpt-t-medical-surgical` — master-slave kinematic mapping for 7-DOF arms, motion scaling (1:1–10:1), 10Hz SIMD recursive notch (tremor) filter (<5us), virtual fixtures: real-time SDF (constant-time trilinear interpolation, <20us) with progressive force feedback + master-slave override on persistent intrusion, co-surgery tool-authority assignment

## Phase 4 — Telesurgery Networking & Integration

- [ ] `tpt-t-medical-teleop` — adapter crate implementing the Domain Teleoperation Interface (DTI) from `tpt-teleop-domain-bridge` (bridge spec.txt §1/§4): `on_teleop_engage`/`on_teleop_disengage`, `ControlCommand` → 7-DOF joint torques with tremor filtering + motion scaling applied, `SensorFeed` packaging (video/haptic/telemetry) back to the operator
- [ ] Predictive display: local UI shows predicted tool position from hand velocity while the remote robot executes actual commands; smooth correction on misprediction; masks network jitter up to 50ms (spec.txt §4.6)
- [ ] Multi-site teleoperation session handling: <10ms end-to-end target, surgeon-in-one-city / patient-in-another topology
- [ ] Safety-state integration: `tpt-t-medical-safety` overrides/vetoes the operator's `ControlCommand` on virtual-fixture violation, consistent with the single-`operator_id` override model in bridge spec.txt §4.1

## Phase 5 — Hospital Logistics & Automation

- [ ] `tpt-t-medical-logistics` — corridor/elevator/door navigation; clean/dirty zone assignment with mandatory UV-sterilize-before-re-entry; MQTT elevator request/single-occupancy arbitration with emergency-transport priority; linens/meals/medication/lab-sample/biohazard transport profiles with infection-control protocols
- [ ] `tpt-t-medical-pharmacy` — unit-dose pick/verify (barcode + weight)/package; refrigerated-storage continuous temperature logging with out-of-range alerting; FEFO (first-expired-first-out) dispensing; narcotic chain-of-custody log entries (biometric/badge auth, timestamp, crypto signature) written through `tpt-t-medical-compliance`
- [ ] `tpt-t-medical-sterilize` — LiDAR room mapping, UV-C dosage calculation from surface distance/reflectivity/shadowing, post-cycle re-scan coverage validation

## Phase 6 — Hospital-IT Interoperability Bridges

- [ ] `tpt-t-medical-bridge-hl7fhir` — HL7v2 ADT feed ingestion (patient admission/transfer/discharge) and FHIR resources (orders, medication requests) in/out, feeding `tpt-t-medical-logistics`/`tpt-t-medical-pharmacy` and posting audit-relevant events to `tpt-t-medical-compliance`
- [ ] `tpt-t-medical-bridge-dicom` — DICOM query/retrieve for pre-op CT/MRI ingestion into `tpt-t-medical-vision`'s segmentation pipeline; DICOM SR export path for structured post-op reports
- [ ] Cross-bridge conformance/integration test suite (HL7/FHIR + DICOM fixtures against both bridge crates)

## Phase 7 — Developer Experience & Simulation

- [ ] `tpt-t-medical-sim` — headless simulator: 7-DOF arm kinematics (gravity/friction/backlash), tissue deformation (mass-spring or FEM), haptic force rendering, network latency/jitter injection, fault injection (sensor failure, motor stall, network drop), 10x real-time execution
- [ ] Visualizer binary (egui, MIT) — 3D arm trajectory + workspace envelope, virtual-fixture no-fly-zone boundaries, haptic force vectors, audit-trail timeline, TMR agreement/disagreement indicators

## Phase 8 — Regulatory & Compliance Documentation Scaffolding

*(Scaffolding only — a real IEC 62304/ISO 14971/ISO 13485 certification requires an actual QMS, external audits, and organizational process beyond this repo; these are the templates a certification effort would build on.)*

- [x] `docs/regulatory/software-development-plan.md` template (IEC 62304 SDLC framing: classification, lifecycle activities, verification/validation strategy)
- [x] `docs/regulatory/risk-management-file.md` template (ISO 14971: hazard identification, risk analysis/evaluation, mitigation traceability — seeded from spec.txt §4/§5.5 safety mechanisms)
- [x] `docs/regulatory/qms-references.md` (ISO 13485 process pointers: where design controls, CAPA, and document control would hook into org-level QMS)
- [x] `docs/regulatory/requirements-traceability-matrix.md` skeleton (spec.txt requirement → crate/module → test evidence)

## Phase 9 — Integration, Hardening & Release

- [ ] End-to-end telesurgery data-flow validation against spec.txt §6 scenario (pre-op planning → patient registration → TMR system check → incision haptics → virtual-fixture engagement → audit trail growth → network-jitter masking → foot-pedal E-stop <10ms halt → post-op log seal/export)
- [ ] Full `cargo-deny` audit pass across entire dependency tree (MIT-chain enforcement, spec.txt §7)
- [ ] Determinism/performance validation: <1ms control-loop latency, 1kHz haptic <50us execution / 1ms deadline, TMR voter <100us disagreement response, SDF <20us, notch+scaling filters <5us, E-stop <10ms
- [ ] Fuzz/property testing for custom zero-allocation code paths (hash-chained WAL, HL7/DICOM parsers, SIMD notch filter, SDF interpolation)
- [ ] Security review: Ed25519/TPM key handling, hash-chain tamper-evidence, HIPAA encryption at rest/in transit, narcotic chain-of-custody biometric-auth spoofing resistance
- [ ] Full documentation pass (`docs/`, crate-level rustdoc, architecture diagrams)
- [ ] Tag v1.0.0 release (pending: commit the workspace, then cut the `v1.0.0` git tag)
