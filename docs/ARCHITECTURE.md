# Architecture

This is a skeleton. Crate responsibilities are taken from
[spec.txt](../spec.txt) §3, and the data flow from spec.txt §6. Each crate
gets a section here once it has an implementation.

## Workspace map

| Group | Crate | Responsibility | Status |
|---|---|---|---|
| Core & surgical control | `tpt-t-medical-core` | Event loop, surgical state machine, lock-free message bus | State machine, tick loop, per-producer lanes and audited wrapper done |
| Core & surgical control | `tpt-t-medical-surgical` | Motion scaling, tremor notch, SDF lookup, fixture force | Signal, geometry, kinematics and virtual-fixture guard done; RT timing pending |
| Edge & perception | `tpt-t-medical-haptic` | 1 kHz impedance rendering, hold-last-value on missed deadline | Contact model and hold logic done; pinned-core timing pending |
| | `tpt-t-medical-vision` | Stereoscopic pipeline, 3D reconstruction, AR overlay | Algorithms done on synthetic data (stereo, overlay, segmentation to distance); real-time and clinical validation pending |
| Hospital logistics | `tpt-t-medical-logistics` | Delivery robots, infection-control zones, elevator arbitration | Zone rules, elevator arbitration, UV-aware routing, MQTT framing, transport profiles and ADT destinations done; door model and broker connection pending |
| Hospital logistics | `tpt-t-medical-pharmacy` | Unit-dose packaging, FEFO dispensing, cold-chain logging | FEFO, cold-chain alerting, custody ledger and barcode/weight packaging done |
| Hospital logistics | `tpt-t-medical-sterilize` | LiDAR room mapping, UV-C dosage, coverage validation | LiDAR room mapping, dose planning and coverage validation done |
| Safety & compliance | `tpt-t-medical-safety` | TMR voter, watchdogs, force limits, workspace envelope, E-stop | Pure logic done; RT timing and core-isolated copies pending |
| | `tpt-t-medical-compliance` | Hash-chained audit log, Ed25519 signatures | Chain, Ed25519, file WAL and block-aligned direct-I/O log done; Linux O_DIRECT, NVMe validation and hardware keys pending |
| | `tpt-t-medical-hipaa` | PHI encryption (AES-256-GCM), consent-token gating | Encryption, consent, revocation and access audit done; hardware key storage pending |
| Edge & perception | `tpt-t-medical-edge` | 1 kHz real-time arm control loop | Torque pipeline (force, gravity, friction, limits, hold-last) done; RT core pending |
| | `tpt-t-medical-track` | Optical, EM and imaging tracking fusion, EKF | Kalman fusion and Horn registration done; nonlinear EKF and sensor validation pending |
| Foundations | `tpt-t-medical-ring` | Wait-free SPSC ring buffer (MPSC deferred) | SPSC core done, loom-verified |
| Telesurgery | `tpt-t-medical-teleop` | Adapter for the `tpt-teleop-domain-bridge` DTI | Adapter done against the DTI, with lifecycle, guard, safety, sensor feed, and two-site link quality; a real transport is pending |
| Bridges | `tpt-t-medical-bridge-hl7fhir` | HL7v2 ADT and FHIR MedicationRequest ingestion | Both parsers, FHIR output, audit, and pharmacy and transport feeds done; other FHIR resources not implemented |
| | `tpt-t-medical-bridge-dicom` | DICOM query/retrieve and SR export | Parser, C-ECHO/C-FIND/C-GET/C-STORE client and SR export done; pixel ingestion pending |
| Tooling | `tpt-t-medical-sim` | Headless simulator with fault injection | Joint dynamics, fault injection, latency and jitter, tissue, tool position and AI fault scenarios done; haptic rendering and visualizer pending |
| Tooling | `tpt-t-medical-ai-session` | AI assist-only session, surgeon-only authority, per-unit audit, fleet requests, PHI gate | Logic done; DTI wiring and TMR vote on AI commands pending |
| Tooling | `tpt-t-medical-mcp` | MCP supervisory interface: read-only fleet tools and one journalled assistance request | Done; no reset, release, limit or veto tool |
| Tooling | `tpt-t-medical-visualizer` | Operator scene: trajectory, envelope, fixture, forces, audit timeline, TMR state | Scene model and projection done; self-contained HTML export |

## Data flow: telesurgery

This is the flow from spec.txt §6, mapped to crates.

```mermaid
flowchart LR
    surgeon["Surgeon master controller"] -->|ControlCommand| link["tpt-teleop link<br/>(<10 ms)"]
    link --> teleop["tpt-t-medical-teleop"]
    teleop --> surgical["tpt-t-medical-surgical<br/>tremor filter, motion scaling,<br/>virtual fixtures (SDF)"]
    surgical --> safety{"tpt-t-medical-safety<br/>TMR voter"}
    safety -->|agree| edge["tpt-t-medical-edge<br/>1 kHz torque loop"]
    safety -->|disagree / E-stop| stop["Safe state (<100 µs)"]
    edge --> arm["7-DOF surgical arm"]
    arm -->|force/torque, pose| haptic["tpt-t-medical-haptic<br/>1 kHz impedance"]
    haptic --> surgeon
    arm -->|video| vision["tpt-t-medical-vision"]
    vision --> surgeon

    surgical -.->|every command and state change| compliance["tpt-t-medical-compliance<br/>signed, hash-chained WAL"]
    safety -.-> compliance
    teleop -.-> compliance
```

## Real-time boundaries

- The haptic and edge loops run on pinned cores with `SCHED_FIFO`. They must
  not allocate or take locks.
- Data moves between the vision/tracking threads and the haptic thread through
  a lock-free SPSC ring buffer.
- Audit writes are asynchronous to the control loops. A failed audit write is
  a safety event, not a silent drop.

## Open decisions

- Ring buffer: decided. `tpt-t-medical-ring` is a copy of the SPSC core from `tpt-t-ring`, not a path dependency on the sister repo. MPSC is deferred.
- Unicode-3.0 is in the `cargo deny` allow list, but spec.txt §2 does not list
  it (see [deny.toml](../deny.toml)).
- Signing library: decided. `ed25519-dalek` (BSD-3-Clause) with `sha2`. `ring` is not used: it is Apache-2.0 AND ISC, which the allow list rejects, and no exception is granted.
- AES-256-GCM: `aes-gcm` 0.10, not 0.11. Version 0.11 pulls in `ctutils` 0.4.2, which has RUSTSEC-2026-0385. The fix (ctutils 0.4.4) needs Rust 1.87, above the workspace MSRV of 1.85. Revisit when the MSRV moves.
