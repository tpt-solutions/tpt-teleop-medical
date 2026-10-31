# Changelog

All notable changes to `tpt-t-medical-teleop` are documented in this file. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
crate uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- The surgical arm as a `DomainTeleopInterface` adapter from the tpt-teleop domain
  bridge, linked by path to `../tpt-teleop/crates/tpt-t-domain-bridge`. Commands
  pass through a 10 Hz tremor notch, motion scaling, and the virtual-fixture guard
  before they move a tool target.
- The tool is tracked by a spring and damper through the edge torque pipeline. The
  safety monitor runs on every tick, and a stop zeroes the torque and latches.
- Engage starts at the current tool position. Disengage ramps the tracking force
  away while gravity compensation holds the arm.
- Tests drive the adapter through `DomainSession`.

### Known issues

- Two-site session handling: a command path and a feed path, each with its own link monitor. The state is the worse of the two, and a lost feed latches the arm. A real transport is not yet implemented.
- `cargo deny check bans` fails on two wildcard path dependencies in the tpt-teleop
  repository, which this crate depends on. Adding `version` to those entries in
  tpt-teleop's workspace clears it.
