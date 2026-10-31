# tpt-t-medical-bridge-dicom

The DICOM bridge for tpt-teleop-medical (Phase 6). It reads the metadata of
uncompressed Part 10 files, runs the calling side of the DICOM network services,
and exports a Basic Text Structured Report.

## Status

Parser, network client and SR export are done. Pixel ingestion is not
implemented, and the SR export has not been conformance-validated.

## Overview

| Module | Main items | Purpose |
|---|---|---|
| root | `DicomHeader`, `TransferSyntax`, `parse`, `ParseError`, `MAX_FILE_BYTES`, `MAX_ELEMENTS` | Metadata parser for Part 10 files |
| `net` | `Association`, `connect`, `echo`, `find`, `get`, `store`, `Command`, `Pdu`, `NetError` | The calling side of C-ECHO, C-FIND, C-GET and C-STORE over TCP |
| `sr` | `export_basic_text_sr`, `SrError`, `MAX_FINDINGS` | Writes a Basic Text SR as a Part 10 file |

### What the parser extracts

- Patient identifier
- Study UID
- Modality
- Image size

These are what the vision pipeline needs to match a scan to a patient. Pixel data
is skipped by its declared length and never read.

### Supported and refused

| Input | Result |
|---|---|
| Explicit VR Little Endian | Supported |
| Implicit VR Little Endian | Supported |
| Compressed transfer syntaxes | Refused |
| Undefined-length elements | Refused. They cannot be skipped safely without parsing their contents. |
| Files over `MAX_FILE_BYTES` (512 MiB) | Refused |
| Malformed input | Returns an error. Does not panic. |

Sequences are skipped by length and not entered.

### Network services

- **C-ECHO, C-FIND, C-GET, C-STORE**: client side, over TCP.
- **C-GET** is used for retrieval. **C-MOVE is not implemented**, because it needs
  an inbound listener.
- The listening (SCP) side is not implemented.
- The tests run the client against a loopback SCP (`tests/loopback.rs`).

Implicit and Explicit VR encoding and PDU framing are in `net`. Message and PDU
sizes are bounded (`MAX_MESSAGE_BYTES`, `MAX_PDU_BYTES`, `FRAGMENT_BYTES`).

## Testing

```sh
cargo test -p tpt-t-medical-bridge-dicom
```

## Limitations

- No pixel ingestion.
- No C-MOVE and no SCP.
- The SR export follows the Basic Text SR structure but has not been validated
  against a conformance test suite.

## Dependencies

None. The crate uses only `std`.

## Related crates

- [`tpt-t-medical-conformance`](../tpt-t-medical-conformance) tests this parser
  together with the HL7 bridge and the audit journal.
- [`tpt-t-medical-vision`](../tpt-t-medical-vision) is where pixel ingestion would
  feed.

## Licence

MIT OR Apache-2.0. See the [workspace README](../../README.md).
