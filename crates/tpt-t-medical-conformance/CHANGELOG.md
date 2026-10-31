# Changelog

All notable changes to `tpt-t-medical-conformance` are documented in this file. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
crate uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Cross-bridge conformance tests, with no product code. They check that an admit
  precedes a transfer or discharge, that a CT scan matches only an admitted patient,
  that every ADT event is journalled and the journal verifies after restart, that
  malformed input from either bridge never panics, and that Debug output hides
  identifiers.
