# Changelog

All notable changes to `tpt-t-medical-surgical` are documented in this file. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
crate uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Motion scaling from 1:1 to 10:1, an RBJ band-stop tremor notch at 10 Hz with
  direct form II transposed state, a trilinear SDF lookup that returns unknown
  outside the grid, and a linear progressive fixture force. A NaN distance gives
  the maximum force.
- 7-DOF forward kinematics with a reference DH table, a predictive display capped
  at a 50 ms horizon, and a virtual-fixture guard that blocks motion into a
  structure and overrides the master on persistent intrusion.

### Known issues

- Timing targets (under 5 µs for the filters, under 20 µs for the SDF lookup) are
  not verified on this target.
