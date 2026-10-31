# Changelog

All notable changes to `tpt-t-medical-edge` are documented in this file. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
crate uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- 1 kHz torque pipeline. It maps tool force to joint torque through the Jacobian,
  adds gravity compensation and friction, clamps to the torque limits, and holds
  the last command on a missed deadline. The virtual-work identities are checked.

### Known issues

- The real-time core (pinned thread, scheduler policy) is not implemented.
