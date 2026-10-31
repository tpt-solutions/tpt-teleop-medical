# Changelog

All notable changes to `tpt-t-medical-visualizer` are documented in this file. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
crate uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Operator scene model: trajectory, workspace box, fixture boundary, force arrows,
  audit timeline, TMR state, and a view projection.
- Self-contained HTML export (`html::render`, with the `export` example). The page
  uses inline SVG and a small script that rotates the view when dragged. It has no
  external resources, and its projection matches the Rust model.

### Known issues

- The interactive window is not in this crate.
