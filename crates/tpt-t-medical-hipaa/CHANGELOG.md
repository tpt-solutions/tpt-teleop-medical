# Changelog

All notable changes to `tpt-t-medical-hipaa` are documented in this file. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
crate uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- PHI encryption with AES-256-GCM. Each record gets a random 96-bit nonce and
  binds the patient identifier as associated data. Consent tokens are signed,
  purpose-limited and expiring. `release` checks consent before decrypting. `Phi`
  redacts its `Debug` output.
- Consent revocation: `RevocationLedger` journals revocations and replays them on
  open. A revoked token is refused even with a valid signature.
- Access audit: `ReleaseAudit` journals every attempt, granted or refused, before
  any plaintext is returned. If the journal write fails, nothing is decrypted.

### Known issues

- Key storage in hardware is not implemented.
