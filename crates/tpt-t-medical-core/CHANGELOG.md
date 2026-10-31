# Changelog

All notable changes to `tpt-t-medical-core` are documented in this file. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
crate uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Surgical state machine (Idle, Registered, Active, Paused, Emergency). Emergency
  is latched and clears only to Idle. Pure logic with no allocation. The
  `EventLoop` drains one SPSC lane per producer on each tick. Lane 0 is the
  emergency lane and is drained first.
- `AuditedLoop`: every transition is journalled before it is applied. Safety
  stops are journalled as a reason record followed by the transition, and they
  take effect even if the journal write fails. Event and state codes are fixed
  for the audit format.
- Central loop wired to the safety decisions and the audit journal.
