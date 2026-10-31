# Changelog

All notable changes to `tpt-t-medical-haptic` are documented in this file. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
crate uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- 1 kHz tick loop with an impedance contact model against a plane. It reads the
  newest sample from the lock-free ring, and holds the last force on a missed
  deadline, a missing or stale sample, or an invalid sample. Force is never zeroed.

### Known issues

- Timing claims (under 50 µs execution, 1 ms deadline on a pinned core) are not
  verified. The crate checks only the logic that decides when to hold.
- No haptic device model.
