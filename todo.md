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
- [ ] `tpt-t-medical-core` — central event loop, surgical state machine (Idle → Registered → Active → Paused → Emergency), lock-free message bus *(state machine, per-producer lanes, emergency-first tick and audited wrapper done)*

## Phase 2 — Safety & Compliance Backbone

- [ ] `tpt-t-medical-safety` *(pure logic done: TMR vote, envelope, force, watchdog, latched E-stop; RT timing and isolated cores pending)* — Triple-Modular Redundancy (TMR) voter across 3 isolated cores, zero-allocation static memory, independent watchdogs, force/torque limiters, workspace envelope enforcement, emergency-stop hierarchies (foot pedal, hand switch, voice, auto), <100us disagreement-to-safe-state
- [ ] `tpt-t-medical-compliance` *(hash chain, Ed25519 signing, file WAL and a block-aligned direct-I/O log done; Linux O_DIRECT, NVMe validation and hardware-backed keys pending)* — append-only hash-chained WAL; events serialized via rkyv, signed Ed25519 (TPM/secure-enclave-backed key), SHA-256 chained, written O_DIRECT to NVMe; tamper-evident (broken chain invalidates log); log export/seal for post-op regulatory archive
- [ ] `tpt-t-medical-hipaa` *(encryption, consent, revocation and access audit done; hardware key storage pending)* — PHI encryption at rest (AES-256-GCM) and in transit; consent-token gating so PHI never enters logs/training data without explicit consent

## Phase 3 — Surgical Control & Haptics

- [ ] `tpt-t-medical-edge` *(torque pipeline done: Cartesian force to joint torque, gravity, friction, limits, hold-last on a missed deadline; RT core pinning pending)* — 1kHz real-time control loop on dedicated RT core, Cartesian → joint-space torque translation, gravity compensation, friction modeling
- [ ] `tpt-t-medical-haptic` *(contact model, hold-last-value and newest-sample read done; pinned core and SCHED_FIFO pending)* — 1kHz haptic rendering, pinned core + SCHED_FIFO, custom impedance control (stiffness/damping/tool-tissue forces), <50us loop / 1ms hard deadline, hold-last-value on missed deadline, lock-free SPSC ring buffer input
- [ ] `tpt-t-medical-track` *(constant-velocity Kalman fusion and Horn registration done, checked on simulated data; nonlinear EKF, portable SIMD and sensor validation pending)* — tool/anatomy tracking fusion (optical NDI-Polaris-style + electromagnetic + intra-op imaging), custom EKF (portable-simd), sub-millimeter registration to pre-op scan (<0.5mm)
- [ ] `tpt-t-medical-vision` *(stereo block matching, depth and point cloud, AR overlay, and segmentation to virtual-fixture distance done, checked on synthetic images; real-time and clinical validation pending)* — stereoscopic endoscope pipeline, 3D reconstruction, depth estimation, pre-op CT/MRI overlay onto live video (AR guidance), critical-structure segmentation ingestion for virtual fixtures *(masks become fixtures through `segmentation`, which resamples to isotropic spacing and calls `volume::signed_distance`; `tests/vision_fixture.rs` in the conformance crate runs a DICOM series through to the surgical guard)*
- [ ] `tpt-t-medical-surgical` *(scaling, notch, SDF lookup, fixture force, forward kinematics and the virtual-fixture guard done; RT timing pending)* — master-slave kinematic mapping for 7-DOF arms, motion scaling (1:1–10:1), 10Hz SIMD recursive notch (tremor) filter (<5us), virtual fixtures: real-time SDF (constant-time trilinear interpolation, <20us) with progressive force feedback + master-slave override on persistent intrusion, co-surgery tool-authority assignment

## Phase 4 — Telesurgery Networking & Integration

- [x] `tpt-t-medical-teleop` — adapter crate implementing the Domain Teleoperation Interface (DTI) from `tpt-teleop-domain-bridge` (bridge spec.txt §1/§4): `on_teleop_engage`/`on_teleop_disengage`, `ControlCommand` → 7-DOF joint torques with tremor filtering + motion scaling applied, `SensorFeed` packaging (video/haptic/telemetry) back to the operator
- [ ] Predictive display *(predictor done: extrapolation capped at the 50 ms horizon, with a correction that does not jump the image; operator UI wiring pending)*: local UI shows predicted tool position from hand velocity while the remote robot executes actual commands; smooth correction on misprediction; masks network jitter up to 50ms (spec.txt §4.6)
- [ ] Multi-site teleoperation session handling: <10ms end-to-end target, surgeon-in-one-city / patient-in-another topology *(link monitor done in `tpt-t-medical-teleop/src/link.rs`: nominal within 10 ms, degraded up to the 50 ms horizon and warned, lost beyond it or on a clock fault, and the adapter latches on lost; a silent link past the horizon also latches. Two-site session handling is modelled: the command and feed paths each have a monitor, the worse of the two sets the state, and `tests/two_site.rs` drives both paths over the simulator's delay link. A real transport and measured WAN latency are still pending)*
- [x] Safety-state integration: `tpt-t-medical-safety` overrides/vetoes the operator's `ControlCommand` on virtual-fixture violation, consistent with the single-`operator_id` override model in bridge spec.txt §4.1 *(the teleop adapter applies the virtual-fixture guard and the safety monitor to every command and every tick; the guard now also checks the commanded target, not only the measured tool, so a lagging tool cannot let the target into a structure)*

## Phase 5 — Hospital Logistics & Automation

- [ ] `tpt-t-medical-logistics` *(zone rules, elevator arbitration, UV-aware route planning, MQTT framing and transport profiles done; door navigation and a live broker connection pending)* — corridor/elevator/door navigation; clean/dirty zone assignment with mandatory UV-sterilize-before-re-entry; MQTT elevator request/single-occupancy arbitration with emergency-transport priority; linens/meals/medication/lab-sample/biohazard transport profiles with infection-control protocols
- [x] `tpt-t-medical-pharmacy` *(FEFO, cold-chain, custody ledger and barcode/weight unit-dose packaging done)* — unit-dose pick/verify (barcode + weight)/package; refrigerated-storage continuous temperature logging with out-of-range alerting; FEFO (first-expired-first-out) dispensing; narcotic chain-of-custody log entries (biometric/badge auth, timestamp, crypto signature) written through `tpt-t-medical-compliance`
- [x] `tpt-t-medical-sterilize` *(dose planning, LiDAR room mapping with shadowing, and coverage check done)* — LiDAR room mapping, UV-C dosage calculation from surface distance/reflectivity/shadowing, post-cycle re-scan coverage validation

## Phase 6 — Hospital-IT Interoperability Bridges

- [x] `tpt-t-medical-bridge-hl7fhir` *(HL7v2 ADT and FHIR MedicationRequest parsers, FHIR output, audit posting to compliance, and the pharmacy and transport feeds done)* — HL7v2 ADT feed ingestion (patient admission/transfer/discharge) and FHIR resources (orders, medication requests) in/out, feeding `tpt-t-medical-logistics`/`tpt-t-medical-pharmacy` and posting audit-relevant events to `tpt-t-medical-compliance`
- [ ] `tpt-t-medical-bridge-dicom` *(metadata parser; C-ECHO, C-FIND, C-GET and C-STORE client; and Basic Text SR export done; pixel ingestion for CT slices and volumes done in `src/pixels.rs`; SR structural conformance checks done in `tests/sr_conformance.rs`; external validator against a PACS pending)* — DICOM query/retrieve for pre-op CT/MRI ingestion into `tpt-t-medical-vision`'s segmentation pipeline; DICOM SR export path for structured post-op reports
- [x] Cross-bridge conformance/integration test suite (HL7/FHIR + DICOM fixtures against both bridge crates)

## Phase 6.5 — AI & Fleet Dispatch

*(Anticipated in bridge spec.txt §4.1 and its closing "future extensions" note; spec.txt §5.6 "AI & Fleet Dispatch" is referenced there but not yet written. AI must never weaken the safety or audit guarantees that apply to a human operator.)*

- [ ] `tpt-t-medical-ai-session` *(state machine, human gating, proposal checks, per-unit audit, fleet requests and PHI gate implemented per the recommendations in docs/design/ai-session.md; TMR vote on AI commands and DTI wiring pending)* — AI controller as a DTI operator: `operator_id` resolves to an AI agent; AUTONOMOUS → REQUESTING_TELEOP → TELEOP_ACTIVE → RETURNING_TO_AUTONOMY transitions; every AI `ControlCommand` passes the same `tpt-t-medical-safety` veto (TMR, envelope, force, E-stop) as a human command; every AI command and handoff logged through `tpt-t-medical-compliance`
- [ ] Human-in-the-loop gating: AI-assist vs full-autonomy modes with explicit human authorization and immediate human override; surgical sessions default to assist-only *(assist-only, surgeon-only authority and immediate override done; full-autonomy mode deliberately not built)*
- [x] Fleet dispatch *(request-only dispatcher surface, per-unit journals, a FIFO queue of units awaiting a surgeon (`Fleet::pending`) and a read-only `Fleet::summary`, as ai-session.md §5 recommends; no authority beyond requesting)*: one dispatcher (AI or human) managing several concurrent DTI sessions (one `operator_id`/session per unit today), with fleet-level orchestration above that
- [x] `tpt-t-medical-mcp` *(read-only tools and one journalled assistance request; no reset, release, limit or veto tool)* — MCP supervisory interface for fleet orchestration: read-mostly, scoped and auditable write tools, no tool can bypass `tpt-t-medical-safety`
- [x] PHI/consent gating for AI and MCP tools: no PHI visible without a `tpt-t-medical-hipaa` consent token; same gate for any training data *(AI reads go through the consent gate, with logging refused; the MCP exposes no patient data, so it needs no consent token)*
- [ ] Simulator coverage (Phase 7): AI-session and fleet scenarios, including misbehaving-AI fault injection *(AI misbehaviour scenarios done; fleet scenarios pending)*

## Phase 7 — Developer Experience & Simulation

- [ ] `tpt-t-medical-sim` *(joint dynamics, fault injection, command latency and jitter, tool kinematics and mass-spring tissue done; haptic rendering done, visualizer pending)* — headless simulator: 7-DOF arm kinematics (gravity/friction/backlash), tissue deformation (mass-spring or FEM), haptic force rendering, network latency/jitter injection, fault injection (sensor failure, motor stall, network drop), 10x real-time execution
- [ ] Visualizer binary *(scene model, projection and a self-contained HTML export done and tested; a live feed from the running system is in `feed` and driven by the simulator in `tests/live.rs`; the operator view is a self-refreshing HTML page (`html::render_live`). egui was dropped: its dependency tree includes Apache-2.0-only crates, which `deny.toml` rejects, so no window toolkit is used)* — 3D arm trajectory + workspace envelope, virtual-fixture no-fly-zone boundaries, haptic force vectors, audit-trail timeline, TMR agreement/disagreement indicators

## Phase 8 — Regulatory & Compliance Documentation Scaffolding

*(Scaffolding only — a real IEC 62304/ISO 14971/ISO 13485 certification requires an actual QMS, external audits, and organizational process beyond this repo; these are the templates a certification effort would build on.)*

- [x] `docs/regulatory/software-development-plan.md` template (IEC 62304 SDLC framing: classification, lifecycle activities, verification/validation strategy)
- [x] `docs/regulatory/risk-management-file.md` template (ISO 14971: hazard identification, risk analysis/evaluation, mitigation traceability — seeded from spec.txt §4/§5.5 safety mechanisms)
- [x] `docs/regulatory/qms-references.md` (ISO 13485 process pointers: where design controls, CAPA, and document control would hook into org-level QMS)
- [x] `docs/regulatory/requirements-traceability-matrix.md` skeleton (spec.txt requirement → crate/module → test evidence)

## Phase 9 — Integration, Hardening & Release

- [ ] End-to-end telesurgery data-flow validation against spec.txt §6 scenario (pre-op planning → patient registration → TMR system check → incision haptics → virtual-fixture engagement → audit trail growth → network-jitter masking → foot-pedal E-stop <10ms halt → post-op log seal/export) *(`tpt-t-medical-teleop/tests/telesurgery_flow.rs` drives the system check, fixture-guarded approach, audit trail, jitter masking, foot-pedal halt and sealed log; pre-op vision, registration and incision haptics are not driven yet, so this stays open)*
- [x] Full `cargo-deny` audit pass across entire dependency tree (MIT-chain enforcement, spec.txt §7) *(all four checks pass on the current tree; re-run on every dependency change)*
- [ ] Determinism/performance validation: <1ms control-loop latency, 1kHz haptic <50us execution / 1ms deadline, TMR voter <100us disagreement response, SDF <20us, notch+scaling filters <5us, E-stop <10ms *(host-measured, ignored-by-default budget tests with `--release --ignored`: TMR tick and disagreement in `tpt-t-medical-safety/tests/budgets.rs`, SDF and notch in `tpt-t-medical-surgical/tests/budgets.rs`, E-stop latch path in the safety suite; control-loop and haptic budgets not yet measured; RT-core certification still pending)*
- [x] Fuzz/property testing for custom zero-allocation code paths (hash-chained WAL, HL7/DICOM parsers, SIMD notch filter, SDF interpolation) *(seeded property suites: `tpt-t-medical-compliance/tests/tamper.rs`, `tpt-t-medical-surgical/tests/properties.rs`, `tpt-t-medical-bridge-hl7fhir/tests/fuzz.rs`, `tpt-t-medical-bridge-dicom/tests/fuzz.rs`; the notch is scalar IIR, not SIMD, so it is covered as-is)*
- [ ] Security review: Ed25519/TPM key handling, hash-chain tamper-evidence, HIPAA encryption at rest/in transit, narcotic chain-of-custody biometric-auth spoofing resistance, AI-session and MCP-interface review (privilege scoping, safety-veto bypass attempts, prompt-injection via tool inputs)
- [x] Full documentation pass (`docs/`, crate-level rustdoc, architecture diagrams) *(README crate table and status refreshed; ARCHITECTURE table refreshed; rustdoc on every public item is enforced by the `missing_docs` lint)*
- [ ] Tag v1.0.0 release (pending: commit the workspace, then cut the `v1.0.0` git tag)
