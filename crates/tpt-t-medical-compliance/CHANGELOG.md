# Changelog

All notable changes to `tpt-t-medical-compliance` are documented in this file.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
the crate uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Append-only hash-chained audit log with SHA-256 digests over sequence, previous
  hash and length-prefixed payload. Verification reports the first broken entry
  and the check that failed.
- Ed25519 signatures from `ed25519-dalek` (BSD-3-Clause) with `verify_strict`.
  The `ring` crate is not used because it is Apache-2.0 AND ISC.
- Journal that persists each entry before it is committed. File-backed WAL with
  length-prefixed framing and a sync after each append. Opening refuses a torn
  tail rather than repairing it, and a bad record or broken chain also stops the
  open.
- `direct::DirectWal`, a block-aligned log. Records are padded to 4 KiB, and the
  reader rejects non-zero padding. On Windows it opens with
  `FILE_FLAG_NO_BUFFERING` and `FILE_FLAG_WRITE_THROUGH`.

### Known issues

- Linux `O_DIRECT` is not set. `DirectWal` is Windows-only.
- Payloads are opaque bytes. rkyv serialisation is not implemented.
- Hardware-backed keys are not implemented.
