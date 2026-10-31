# Changelog

All notable changes to `tpt-t-medical-track` are documented in this file. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
crate uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Constant-velocity Kalman filter fusing optical and electromagnetic position,
  with outlier gating, and Horn rigid registration to a pre-op scan.

### Known issues

- Checked on simulated data only.
- Linear measurement models only. Nonlinear sensor models are not implemented.
