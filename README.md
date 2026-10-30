# tpt-teleop-medical

Safety-critical, zero-bloat Rust middleware for telesurgery, hospital
logistics, automated pharmacy, and sterilization robotics.

> **Status: pre-alpha.** The workspace scaffolding exists; no crates have been
> implemented yet. See [todo.md](todo.md) for the phased checklist.

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
