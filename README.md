# tpt-teleop-medical

Safety-critical, zero-bloat Rust middleware for telesurgery, hospital
logistics, automated pharmacy, and sterilization robotics.

> **Status: pre-alpha.** Pure logic is implemented and tested for most of the
> safety, compliance, bridge and hospital-automation crates. Hardware
> integration is not: there is no real-time target, no O_DIRECT WAL, no
> hardware-backed keys, no arm geometry, and no teleoperation adapter. Nothing
> here should be used on a patient. See [todo.md](todo.md) for the full
> checklist.

## Crates

| Crate | What it does | Status |
|---|---|---|
| `tpt-t-medical-ring` | Wait-free SPSC ring buffer | Done; loom-checked |
| `tpt-t-medical-core` | Surgical state machine, tick loop, audited transitions | Logic done |
| `tpt-t-medical-compliance` | Hash-chained, Ed25519-signed journal; file WAL | Logic done; no O_DIRECT |
| `tpt-t-medical-safety` | TMR vote, envelope, force, watchdog, latched E-stop | Logic done; no RT timing |
| `tpt-t-medical-hipaa` | AES-256-GCM PHI, consent, revocation, access audit | Logic done; no hardware keys |
| `tpt-t-medical-surgical` | Motion scaling, tremor notch, SDF fixtures | Signal and geometry done |
| `tpt-t-medical-haptic` | Impedance contact, hold-last-value loop | Logic done; no pinned core |
| `tpt-t-medical-sim` | Joint dynamics and fault injection | Headless core only |
| `tpt-t-medical-pharmacy` | FEFO, cold chain, narcotic custody | Logic done |
| `tpt-t-medical-sterilize` | UV-C dose planning, coverage check | Logic done; no LiDAR |
| `tpt-t-medical-logistics` | Clean/dirty zones, elevator arbitration | Logic done; no path planning |
| `tpt-t-medical-bridge-hl7fhir` | HL7v2 ADT, FHIR MedicationRequest, audit posting | Parsers and audit done; MedicationRequest wired to pharmacy; ADT not yet wired to logistics |
| `tpt-t-medical-bridge-dicom` | Uncompressed DICOM metadata | Parser done; no network services |
| `tpt-t-medical-conformance` | Cross-bridge tests (test-only) | Done |

Not started: edge control, tracking, vision, teleop, the AI and MCP layer, and
the visualizer. See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

## What it is

tpt-teleop-medical is a Cargo workspace of small, single-purpose crates, all
prefixed `tpt-t-medical-*`. The repository keeps the full name
`tpt-teleop-medical`. Design detail lives in:

- [spec.txt](spec.txt): the system design (crates, latency budgets, safety
  mechanisms, audit trail, feature matrix).
- [bridge spec.txt](bridge%20spec.txt): how this repo plugs into the shared
  `tpt-teleop` core through the Domain Teleoperation Interface.
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): workspace map and data flow.

## Hard constraints

These come from the spec and apply to every crate:

- Deterministic control paths: 1 kHz haptic loop, <1 ms control latency, <10 ms
  end-to-end telesurgery latency.
- No heap allocation on real-time paths.
- Every safety-critical command is logged to the tamper-evident audit trail.
- Dependencies are limited to MIT-compatible licenses (`cargo deny` enforces
  this; see [deny.toml](deny.toml)).

## Building

The toolchain is pinned to stable with `rustfmt` and `clippy` by
[rust-toolchain.toml](rust-toolchain.toml). The minimum supported Rust version
is 1.85.

```sh
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo deny check
```

## Licensing

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your
option. Copyright TPT Solutions.

Dependencies are accepted only through their MIT option; see
[deny.toml](deny.toml).

Deploying this software on a medical device is a separate regulatory effort.
The license does not cover IEC 62304, ISO 14971 or ISO 13485 certification.
See [docs/regulatory/](docs/regulatory/) for scaffolding templates only.

## Security

To report a vulnerability, follow [SECURITY.md](SECURITY.md). Do not open a
public issue for it.
