//! Binary encoding of [`Entry`] records.
//!
//! Fields are written in a fixed order, little-endian, with no serde:
//!
//! | Field       | Size                  |
//! |-------------|-----------------------|
//! | `seq`       | 8 bytes               |
//! | `prev`      | 32 bytes              |
//! | `hash`      | 32 bytes              |
//! | `sig_len`   | 2 bytes               |
//! | `signature` | `sig_len` bytes       |
//! | `pay_len`   | 4 bytes               |
//! | `payload`   | `pay_len` bytes       |
//!
//! rkyv zero-copy encoding is the spec's target (spec.txt §4.4). This simple
//! format stands in until that is decided. Decoding is strict: any leftover
//! byte is an error.

use std::io;

use crate::Entry;

/// Encodes an entry.
///
/// # Errors
///
/// Returns `InvalidInput` if the signature or payload is too large for its
/// length field.
pub(crate) fn encode(entry: &Entry) -> io::Result<Vec<u8>> {
    let sig_len = u16::try_from(entry.signature.len()).map_err(|_| too_large("signature"))?;
    let pay_len = u32::try_from(entry.payload.len()).map_err(|_| too_large("payload"))?;

    let mut out =
        Vec::with_capacity(8 + 32 + 32 + 2 + entry.signature.len() + 4 + entry.payload.len());
    out.extend_from_slice(&entry.seq.to_le_bytes());
    out.extend_from_slice(&entry.prev);
    out.extend_from_slice(&entry.hash);
    out.extend_from_slice(&sig_len.to_le_bytes());
    out.extend_from_slice(&entry.signature);
    out.extend_from_slice(&pay_len.to_le_bytes());
    out.extend_from_slice(&entry.payload);
    Ok(out)
}

/// Decodes an entry. Returns `None` if the bytes are truncated, malformed, or
/// have trailing data.
pub(crate) fn decode(bytes: &[u8]) -> Option<Entry> {
    let mut cur = Cursor { bytes, pos: 0 };
    let seq = u64::from_le_bytes(cur.take()?);
    let prev = cur.take::<32>()?;
    let hash = cur.take::<32>()?;
    let sig_len = u16::from_le_bytes(cur.take()?);
    let signature = cur.slice(usize::from(sig_len))?.to_vec();
    let pay_len = u32::from_le_bytes(cur.take()?);
    let payload = cur.slice(usize::try_from(pay_len).ok()?)?.to_vec();
    if cur.pos != bytes.len() {
        return None;
    }
    Some(Entry {
        seq,
        prev,
        payload,
        hash,
        signature,
    })
}

fn too_large(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, format!("{what} too large"))
}

struct Cursor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn slice(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let out = self.bytes.get(self.pos..end)?;
        self.pos = end;
        Some(out)
    }

    fn take<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.slice(N)?.try_into().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Entry {
        Entry {
            seq: 42,
            prev: [1; 32],
            payload: b"emergency-stop".to_vec(),
            hash: [2; 32],
            signature: vec![3; 64],
        }
    }

    #[test]
    fn round_trips() {
        let entry = sample();
        let bytes = encode(&entry).expect("encodes");
        assert_eq!(decode(&bytes), Some(entry));
    }

    #[test]
    fn every_truncation_is_rejected() {
        let bytes = encode(&sample()).expect("encodes");
        for len in 0..bytes.len() {
            assert_eq!(decode(&bytes[..len]), None, "truncated to {len} bytes");
        }
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        let mut bytes = encode(&sample()).expect("encodes");
        bytes.push(0);
        assert_eq!(decode(&bytes), None);
    }

    #[test]
    fn empty_payload_and_signature_round_trip() {
        let entry = Entry {
            seq: 0,
            prev: [0; 32],
            payload: Vec::new(),
            hash: [0; 32],
            signature: Vec::new(),
        };
        let bytes = encode(&entry).expect("encodes");
        assert_eq!(decode(&bytes), Some(entry));
    }
}
