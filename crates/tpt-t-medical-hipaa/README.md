# tpt-t-medical-hipaa

HIPAA-oriented handling of patient data for tpt-teleop-medical: AES-256-GCM
encryption of PHI, signed and purpose-limited consent tokens, journalled
revocation, and an audit of every access attempt (spec.txt §3, §4.4).

## Status

Logic done for encryption, consent, revocation and access audit. Key storage in
hardware is not implemented. This crate does not make a system HIPAA-compliant
by itself; see the regulatory notes in [docs/regulatory/](../../docs/regulatory/).

## Overview

| Module | Items | Purpose |
|---|---|---|
| `phi` | `Phi`, `PhiKey`, `seal`, `open`, `seal_for_patient`, `expose`, `PhiError` | AES-256-GCM encryption. Each record gets a random 96-bit nonce and binds the patient identifier as associated data. `Phi` redacts its `Debug` output. |
| `consent` | `ConsentToken`, `Purpose`, `issue`, `check`, `ConsentError` | Signed, purpose-limited, expiring consent. `Purpose` is `Treatment`, `Logging` or `Training`. |
| `revocation` | `RevocationLedger`, `RevocationSet`, `revoke`, `check_with_revocations`, `RevocationError` | Revocations are journalled and replayed on open. A revoked token is refused even when its signature is valid. |
| `release_audit` | `ReleaseAudit`, `AuditedReleaseError` | Journals every attempt, granted or refused, before any plaintext is returned |
| root | `release`, `Request`, `ReleaseError` | The unaudited building block. It checks consent, then decrypts only if consent passes. |

Constants: `KEY_LEN`, `NONCE_LEN`, `TAG_LEN`.

### Release flow

Production code should use `ReleaseAudit`, not `release` on its own:

1. The attempt is journalled, granted or refused.
2. If the journal write fails, nothing is decrypted.
3. Only then is consent checked, and plaintext returned if it passes.

Consent is checked before decryption, and revocations are checked alongside the
signature.

## Testing

```sh
cargo test -p tpt-t-medical-hipaa
```

## Limitations

- Keys are held in memory. There is no hardware key storage.
- Encryption is only as sound as key handling around it. Callers are responsible
  for key lifecycle.

## Dependencies

- `aes-gcm` 0.10 (see the note on the version choice in
  [docs/ARCHITECTURE.md](../../docs/ARCHITECTURE.md))
- `getrandom` 0.4 (nonces)
- `sha2` 0.10
- [`tpt-t-medical-compliance`](../tpt-t-medical-compliance) (journals)

## Related crates

- [`tpt-t-medical-ai-session`](../tpt-t-medical-ai-session) reads PHI for the AI
  only through this crate's consent gate.
- [`tpt-t-medical-compliance`](../tpt-t-medical-compliance) stores the audit and
  revocation journals.

## Licence

MIT OR Apache-2.0. See the [workspace README](../../README.md).
