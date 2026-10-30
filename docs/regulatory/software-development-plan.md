# Software Development Plan (IEC 62304 template)

> **Scaffolding only.** This template is a starting point for a certification
> effort. It is not a certified plan. Certification needs an actual QMS,
> external audit, and organisational process beyond this repository.

Document ID: `SDP-TPT-MED-001`
Version: `0.1-draft`
Owner: `[name]`
Approved by: `[name, role, date]`

## 1. Purpose and scope

- Software items covered: the `tpt-t-medical-*` crates listed in
  [../ARCHITECTURE.md](../ARCHITECTURE.md).
- Intended use: `[e.g. teleoperated surgical arm control, UV-C disinfection]`.
- Out of scope: `[e.g. the tpt-teleop transport, owned by the sister repo]`.

## 2. Software safety classification (IEC 62304 §4.3)

| Software item | Classification (A / B / C) | Rationale |
|---|---|---|
| `tpt-t-medical-safety` | C | Failure could cause death or serious injury |
| `tpt-t-medical-edge` | C | Controls arm torque |
| `tpt-t-medical-haptic` | `[B or C]` | Loss of force feedback can cause tissue damage |
| `tpt-t-medical-compliance` | `[B]` | Audit failure invalidates regulatory evidence |
| `tpt-t-medical-hipaa` | `[B]` | PHI exposure is a privacy harm, not a direct injury |
| `tpt-t-medical-sim` | A | Development tool, not in the device |

Classification is by the worst credible harm. Each row is confirmed by
the risk file in [risk-management-file.md](risk-management-file.md).

## 3. Software lifecycle model

Lifecycle model: `[e.g. iterative, with V-model verification per release]`.

| Activity (IEC 62304 clause) | Artifact | Owner | Where it lives |
|---|---|---|---|
| Planning (§5.1) | This document | `[ ]` | `docs/regulatory/` |
| Requirements analysis (§5.2) | Spec and requirements IDs | `[ ]` | `spec.txt`, `bridge spec.txt` |
| Architecture (§5.3) | Architecture document | `[ ]` | `docs/ARCHITECTURE.md` |
| Detailed design (§5.4) | Crate-level rustdoc | `[ ]` | Source |
| Implementation (§5.5) | Source and code review | `[ ]` | Git history |
| Integration and testing (§5.6) | Test reports | `[ ]` | `[ci artifacts]` |
| System testing (§5.7) | Simulation and bench reports | `[ ]` | `[ ]` |
| Release (§5.8) | Tagged release and SBOM | `[ ]` | GitHub releases |
| Maintenance (§6) | Change requests, CHANGELOG | `[ ]` | `CHANGELOG.md` |

## 4. Verification and validation strategy

- **Unit tests** cover every function on a safety-critical path.
- **Property tests and fuzzing** cover the hash-chained WAL, the HL7/DICOM
  parsers, the notch filter and SDF interpolation (todo.md, Phase 9).
- **Timing tests** check each deadline in spec.txt §4: 1 kHz haptic loop,
  <100 µs TMR disagreement response, <20 µs SDF, <5 µs filters, <10 ms E-stop.
- **Fault injection** via `tpt-t-medical-sim`: sensor failure, motor stall,
  network drop.
- **Hazard-based tests**: each hazard in the risk file maps to at least one
  test. See the traceability matrix.

## 5. Configuration management

- Git, with `master` protected and every change through a reviewed PR.
- Tagged releases, starting at `v1.0.0`.
- Dependencies are gated by `cargo deny` (see [../../deny.toml](../../deny.toml)).

## 6. Problem resolution

- Defects are logged as issues with a severity class: safety, functional, or
  cosmetic.
- Safety-class defects follow the CAPA process in the QMS. See
  [qms-references.md](qms-references.md).

## 7. Roles and responsibilities

| Role | Holder |
|---|---|
| Software lead | `[ ]` |
| Safety engineer | `[ ]` |
| Verification lead | `[ ]` |
| Regulatory affairs | `[ ]` |

## 8. Revision history

| Version | Date | Author | Change |
|---|---|---|---|
| 0.1-draft | `[ ]` | `[ ]` | Initial template |
