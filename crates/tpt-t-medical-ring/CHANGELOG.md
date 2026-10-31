# Changelog

All notable changes to `tpt-t-medical-ring` are documented in this file. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the crate
uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Wait-free SPSC ring buffer (`SpscRing`), adapted from the tpt-teleop `tpt-t-ring`
  crate. Unused modules (shared memory, pointer rings, byte casting, layout) are
  left out. Unsafe code is confined to one module.
- Verified with unit tests and loom model checks. The loom tests include a
  deliberately weakened ordering that they catch.
- CI: a loom model-check job for this crate.
- CI: a Miri job that runs the crate's single-threaded unit tests under nightly
  Miri.

### Known issues

- The multi-producer variant (MPSC) is deferred.
