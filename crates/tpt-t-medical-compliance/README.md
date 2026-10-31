# tpt-t-medical-compliance

An append-only, hash-chained, signed audit log for surgical events
(spec.txt §4.4). Every other crate that records a safety-relevant decision
writes it here.

## Status

Logic done. The hash chain, Ed25519 signing, file-backed write-ahead log (WAL)
and a block-aligned direct-I/O log are implemented. Linux `O_DIRECT`, NVMe
validation and hardware-backed keys are not.

## Overview

Each entry commits to the one before it. An entry's SHA-256 digest covers its
sequence number, the previous digest and the length-prefixed payload. Changing,
removing or reordering any entry breaks verification at the first affected
position.

### Main types

| Item | Module | Purpose |
|---|---|---|
| `Entry` | root | A sequence number, previous hash, payload, hash and signature |
| `Log` | root | The in-memory chain. `append`, `verify`, `verify_entries`, `from_entries` |
| `Journal<S: Storage>` | `journal` | A log that persists each entry before it counts as committed |
| `Storage` | `journal` | The persistence trait |
| `Signer`, `Verifier` | root | Pluggable signing traits |
| `Ed25519Signer`, `Ed25519Verifier` | `ed25519` | Ed25519 backend built on `ed25519-dalek` |
| `FileWal` | `wal` | File-backed WAL with length-prefixed framing and a sync after each append |
| `DirectWal` | `direct` | Block-aligned log. Records are padded to 4 KiB (`BLOCK`). |
| `ChainError`, `ChainErrorKind` | root | Reports the first broken entry and the check that failed |
| `GENESIS`, `Hash` | root | The chain's starting value and the 32-byte digest type |

### Persistence

- `FileWal` refuses to open a torn tail rather than repairing it. A bad record
  or a broken chain also stops the open.
- `DirectWal` is Windows-only at present. It opens with
  `FILE_FLAG_NO_BUFFERING` and `FILE_FLAG_WRITE_THROUGH`. The reader rejects
  non-zero padding.

### Signing

Signatures use `ed25519-dalek` (BSD-3-Clause) with `verify_strict`.
The `ring` crate is not used, because it is Apache-2.0 AND ISC and the
workspace licence allow-list rejects that combination (see
[deny.toml](../../deny.toml)).

## Design constraints

- The log allocates. Appends belong on a logging thread, not on the real-time
  control path.
- Payloads are opaque bytes. Serialisation of typed payloads (rkyv) is not done.

## Testing

```sh
cargo test -p tpt-t-medical-compliance
```

## Limitations

- No `O_DIRECT` on Linux.
- No hardware-backed keys (TPM, secure enclave).
- Payloads are not typed.

## Dependencies

- `ed25519-dalek` 3 (signatures)
- `sha2` 0.10 (digests)

## Related crates

Used by [`tpt-t-medical-core`](../tpt-t-medical-core),
[`tpt-t-medical-hipaa`](../tpt-t-medical-hipaa),
[`tpt-t-medical-pharmacy`](../tpt-t-medical-pharmacy),
[`tpt-t-medical-ai-session`](../tpt-t-medical-ai-session) and
[`tpt-t-medical-bridge-hl7fhir`](../tpt-t-medical-bridge-hl7fhir).

## Licence

MIT OR Apache-2.0. See the [workspace README](../../README.md).
