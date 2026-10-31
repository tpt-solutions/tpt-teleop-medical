# Changelog

All notable changes to `tpt-t-medical-ai-session` are documented in this file. The
format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the
crate uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- AI controller as an assist-only operator, following `docs/design/ai-session.md`.
  The AI can only propose, and each proposal is checked for envelope, force, a
  contradiction with the operator's motion, and rate.
- Only the operating surgeon can engage, release, override, renew or reset.
  Sessions expire, a missed heartbeat is a fault, and a session opens latched.
- Every command, refusal and state change is journalled through
  `tpt-t-medical-compliance`.
- Fleet support: assistance requests only reach dispatchers, with one journal per
  unit.
- AI reads of PHI go through the HIPAA consent gate, and logging is refused.

### Known issues

- Wiring to the tpt-teleop domain interface is pending.
- A TMR vote on AI commands is pending.
