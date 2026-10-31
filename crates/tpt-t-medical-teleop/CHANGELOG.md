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
- `Adapter::on_autonomous_motion` applies a displacement the AI session has
  accepted in full-autonomy mode. It skips the tremor notch and motion scaler,
  which act on operator input, and goes through the same virtual-fixture guard and
  latch checks as operator motion. It is refused unless a surgeon has engaged.
- An end-to-end test drives an autonomous `tpt-t-medical-ai-session` through the
  adapter.
- A network transport (`transport`) between the surgeon and the arm. It has two
  sockets, one for commands and one for telemetry. Each has a signed handshake
  against the trust store, and then sealed datagrams under the negotiated session.
  The arm refuses a live takeover and any command whose sequence number does not
  rise.

### Known issues

- Two-site session handling: a command path and a feed path, each with its own link monitor. The state is the worse of the two, and a lost feed latches the arm. Measured WAN latency is not yet recorded.
- The transport is loopback-tested only. Its one-way latency figures assume a shared disciplined clock, and nothing here checks that clock.
- `cargo deny check bans` fails on two wildcard path dependencies in the tpt-teleop
  repository, which this crate depends on. Adding `version` to those entries in
  tpt-teleop's workspace clears it.
