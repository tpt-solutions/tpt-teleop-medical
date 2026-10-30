# Changelog

All notable changes to this project are documented in this file. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `tpt-t-medical-core`: surgical state machine (Idle, Registered, Active, Paused, Emergency). Emergency is latched and clears only to Idle. Pure logic with no allocation. The event loop and message bus are not yet implemented.
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
