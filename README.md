# tpt-teleop-medical

Safety-critical, zero-bloat Rust middleware for telesurgery, hospital
logistics, automated pharmacy, and sterilization robotics.

> **Status: pre-alpha.** Pure logic is implemented and tested for most of the
> safety, compliance, bridge and hospital-automation crates. Hardware
> integration is not: there is no real-time target, no Linux O_DIRECT WAL, no
> hardware-backed keys, no hardware-validated arm kinematics, and no real two-site
> transport. Nothing here should be used on a patient. See
> [todo.md](todo.md) for the full checklist.

## Crates

| Crate | What it does | Status |
|---|---|---|
| `tpt-t-medical-ring` | Wait-free SPSC ring buffer | Done; loom-checked |
| `tpt-t-medical-core` | Surgical state machine, tick loop, audited transitions | Logic done |
| `tpt-t-medical-compliance` | Hash-chained, Ed25519-signed journal; file WAL; block-aligned direct-I/O log | Logic done; Windows direct I/O only; no hardware keys |
| `tpt-t-medical-safety` | TMR vote, envelope, force, watchdog, latched E-stop | Logic done; no RT timing |
| `tpt-t-medical-hipaa` | AES-256-GCM PHI, consent, revocation, access audit | Logic done; no hardware keys |
| `tpt-t-medical-surgical` | Motion scaling, tremor notch, SDF fixtures, 7-DOF kinematics, predictive display, virtual-fixture guard | Signal, geometry and guard logic done; no RT timing |
| `tpt-t-medical-edge` | 1 kHz torque pipeline: Jacobian force mapping, gravity, friction, limits, hold-last | Logic done; no RT core |
| `tpt-t-medical-track` | Constant-velocity Kalman fusion; Horn rigid registration | Logic done; linear model; checked on simulated data |
| `tpt-t-medical-ai-session` | AI assist-only session, surgeon-only authority, per-unit audit, fleet requests, PHI gate | Logic done; no DTI wiring; MCP deferred |
| `tpt-t-medical-haptic` | Impedance contact, hold-last-value loop | Logic done; no pinned core |
| `tpt-t-medical-sim` | Joint dynamics, fault injection, latency and jitter, tissue, tool position, AI fault scenarios | Headless core; haptic rendering on the impedance stand-in; no visualizer |
| `tpt-t-medical-pharmacy` | FEFO, cold chain, narcotic custody, barcode/weight unit-dose packaging | Logic done |
| `tpt-t-medical-sterilize` | UV-C dose planning, LiDAR room mapping with shadowing, coverage check | Logic done |
| `tpt-t-medical-logistics` | Clean/dirty zones, elevator arbitration, UV-aware routing, MQTT framing, transport profiles | Logic done; no broker connection, no door model |
| `tpt-t-medical-teleop` | Telesurgery adapter for the tpt-teleop `DomainTeleopInterface`: engage, command, disengage, emergency stop, sensor feed | Adapter done against the DTI; two-site link quality on both paths; no real transport |
| `tpt-t-medical-vision` | Stereo depth, point cloud, AR overlay, segmentation to fixture distance (`segmentation`) | Algorithms done on synthetic data; no real-time or clinical validation |
| `tpt-t-medical-mcp` | MCP supervisory interface: read-only tools and one journalled assistance request | Done; no reset, release, limit or veto tool |
| `tpt-t-medical-visualizer` | Scene model: trajectory, envelope, fixture, forces, timeline, TMR state, projection | Scene model and projection done; self-contained HTML export (`examples/export.rs`) |
| `tpt-t-medical-bridge-hl7fhir` | HL7v2 ADT, FHIR MedicationRequest, audit posting, pharmacy and transport feeds | Parsers, audit and both feeds done |
| `tpt-t-medical-bridge-dicom` | Uncompressed DICOM metadata; C-ECHO, C-FIND, C-GET, C-STORE client; Basic Text SR export | Parser, client, SR export and CT pixel ingestion done; SR not conformance-validated |
| `tpt-t-medical-conformance` | Cross-bridge tests (test-only) | Done |

The teleop adapter links to the tpt-teleop domain bridge (`tpt-teleop/crates/tpt-t-domain-bridge`) by path. Not started: a real two-site transport, and a haptic device model beyond the impedance stand-in. The live feed and the HTML operator view exist. Pixel ingestion now feeds segmentation to fixtures; the dispatcher UI is pending. See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md).

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

The minimum supported Rust version is 1.85. CI builds and tests on Linux and
Windows with stable and 1.85, and runs loom and Miri checks on `tpt-t-medical-ring`.

`tpt-t-medical-teleop` depends on the tpt-teleop core by relative path, so the
workspace expects a sibling checkout next to this one:

```
parent/
├── tpt-teleop/          # https://github.com/tpt-solutions/tpt-teleop
└── tpt-teleop-medical/  # this repository
```

```sh
cargo build --workspace
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo deny check
```

`cargo deny check bans` currently fails on two wildcard path dependencies in the
tpt-teleop workspace. See the Known issues section of
[CHANGELOG.md](CHANGELOG.md).

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
