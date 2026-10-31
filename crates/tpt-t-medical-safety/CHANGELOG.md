# Changelog

All notable changes to `tpt-t-medical-safety` are documented in this file. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
crate uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Safety monitor: workspace envelope, force limit (NaN refused), watchdog (gaps
  and clock regressions), latched emergency stop from four sources, and a TMR
  voter that requires all three copies to agree. Stops latch until an explicit
  reset. Timing targets are not verified.
- `Limits::watchdog_timeout_us`, so a session can check its heartbeat against the
  watchdog.

### Known issues

- Timing targets (under 100 µs disagreement response, under 10 ms E-stop) are
  not verified.
- Core isolation for the three TMR copies is not implemented.
