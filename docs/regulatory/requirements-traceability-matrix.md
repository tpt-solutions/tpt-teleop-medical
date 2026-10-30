# Requirements Traceability Matrix (skeleton)

Traces each requirement from [spec.txt](../../spec.txt) to the crate or module
that implements it and to the test that verifies it.

Columns:

- **Req ID** — `SPEC-<section>-<n>`, assigned from the spec section it comes from.
- **Requirement** — short statement, quoted or paraphrased from the spec.
- **Crate / module** — where the requirement is implemented. `—` until built.
- **Test evidence** — test name(s) or CI job that verifies it. `—` until written.
- **Status** — `Not started`, `Implemented`, or `Verified`.

| Req ID | Requirement | Crate / module | Test evidence | Status |
|---|---|---|---|---|
| SPEC-4.1-1 | Three independent safety monitors on isolated cores | `tpt-t-medical-safety` | — | Not started |
| SPEC-4.1-2 | Voter enters safe state within 100 µs on disagreement | `tpt-t-medical-safety` | — | Not started |
| SPEC-4.1-3 | Zero allocation on safety paths | `tpt-t-medical-safety` | — | Not started |
| SPEC-4.2-1 | 1 kHz haptic loop, <50 µs execution, 1 ms deadline | `tpt-t-medical-haptic` | — | Not started |
| SPEC-4.2-2 | Hold last value on missed deadline | `tpt-t-medical-haptic` | — | Not started |
| SPEC-4.2-3 | Lock-free SPSC input from tracking thread | `tpt-t-medical-ring` | — | Not started |
| SPEC-4.3-1 | SDF constant-time trilinear lookup, <20 µs | `tpt-t-medical-surgical` | — | Not started |
| SPEC-4.3-2 | Progressive force, then override on persistent intrusion | `tpt-t-medical-surgical` | — | Not started |
| SPEC-4.4-1 | Events rkyv-serialized, Ed25519-signed, SHA-256 chained | `tpt-t-medical-compliance` | — | Not started |
| SPEC-4.4-2 | Broken chain invalidates the log | `tpt-t-medical-compliance` | — | Not started |
| SPEC-4.5-1 | 10 Hz notch filter, <5 µs | `tpt-t-medical-surgical` | — | Not started |
| SPEC-4.5-2 | Motion scaling 1:1 to 10:1 | `tpt-t-medical-surgical` | — | Not started |
| SPEC-4.6-1 | Predictive display masks up to 50 ms jitter | `tpt-t-medical-teleop` | — | Not started |
| SPEC-4.6-2 | End-to-end latency under 10 ms | `tpt-t-medical-teleop` | — | Not started |
| SPEC-5.2-1 | Clean/dirty zones with mandatory UV sterilization | `tpt-t-medical-logistics` | — | Not started |
| SPEC-5.2-2 | Single-occupancy elevator arbitration, emergency priority | `tpt-t-medical-logistics` | — | Not started |
| SPEC-5.2-3 | Narcotic chain-of-custody, biometric auth, signed | `tpt-t-medical-pharmacy` | — | Not started |
| SPEC-5.3-1 | Barcode and weight verification | `tpt-t-medical-pharmacy` | — | Not started |
| SPEC-5.3-2 | FEFO dispensing | `tpt-t-medical-pharmacy` | — | Not started |
| SPEC-5.4-1 | UV-C dose from distance, reflectivity, shadowing | `tpt-t-medical-sterilize` | — | Not started |
| SPEC-5.4-2 | Post-cycle coverage validation | `tpt-t-medical-sterilize` | — | Not started |
| SPEC-5.5-1 | Workspace envelope enforced | `tpt-t-medical-safety` | — | Not started |
| SPEC-5.5-2 | Force/torque limit triggers immediate stop | `tpt-t-medical-safety` | — | Not started |
| SPEC-5.5-3 | Independent E-stop paths; halt in <10 ms | `tpt-t-medical-safety` | — | Not started |
| SPEC-5.5-4 | Power loss: hold or retract to safe pose | `tpt-t-medical-edge` | — | Not started |
| SPEC-8-1 | Headless simulator with fault injection, 10x real-time | `tpt-t-medical-sim` | — | Not started |

Add a row for each requirement as it is written in the spec. A requirement
without a test evidence entry cannot be marked `Verified`.
