# Changelog

All notable changes to `tpt-t-medical-sim` are documented in this file. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the crate
uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Headless simulator core for a seven-joint arm at a fixed 1 kHz step. Each joint
  has gravity, viscous and Coulomb friction, and a backlash operator on its
  reported position.
- Fault injection and clearing: sensor failure, motor stall and network drop.
  Non-finite commands are refused. Runs are deterministic.
- Deterministic command latency and jitter.
- A mass-spring tissue model, and tool position from the joint readings.
- An integration test of five AI fault scenarios against `tpt-t-medical-ai-session`.

### Known issues

- Haptic rendering is not modelled.
- The visualizer is not started.
