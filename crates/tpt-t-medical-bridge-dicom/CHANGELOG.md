# Changelog

All notable changes to `tpt-t-medical-bridge-dicom` are documented in this file.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the crate uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Bounded metadata parser for uncompressed Part 10 files (Explicit and Implicit VR
  Little Endian). It extracts the patient ID, study UID, modality and image size.
  Pixel data is skipped by length. Compressed syntaxes, undefined-length elements
  and files over the size limit are refused.
- DICOM network client over TCP (`net` module), with Implicit and Explicit VR
  encoding, PDU framing, and the C-ECHO, C-FIND, C-GET and C-STORE operations.
  C-GET is used for retrieval, because C-MOVE needs an inbound listener. Tests run
  the client against a loopback SCP.
- `sr` exports a Basic Text Structured Report as a Part 10 file.

### Known issues

- Pixel data is not ingested.
- C-MOVE and the listening (SCP) side are not implemented.
- The SR export has not been conformance-validated.
