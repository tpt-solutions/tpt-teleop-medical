# Changelog

All notable changes to `tpt-t-medical-vision` are documented in this file. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
crate uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Block-matching stereo with a left-right check, depth from disparity,
  back-projection to a point cloud, AR overlay of scan points through the
  registration, and segmentation to a signed-distance grid for the virtual
  fixtures, using an exact separable distance transform.

### Known issues

- Checked on synthetic images only.
- Not tuned for speed. The latency targets need the target hardware.
