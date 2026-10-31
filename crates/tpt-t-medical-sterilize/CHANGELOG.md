# Changelog

All notable changes to `tpt-t-medical-sterilize` are documented in this file. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
crate uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- UV-C dose planning from geometry, and a post-cycle coverage check. Reflection is
  not credited.
- LiDAR room mapping. Returns are binned into a voxel grid, and lit faces are found
  with shadowing. The faces feed the existing dose planner.

### Known issues

- Reflectance is recorded but not used in the dose.
