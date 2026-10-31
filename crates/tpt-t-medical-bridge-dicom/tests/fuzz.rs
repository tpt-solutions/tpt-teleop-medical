//! Robustness and round-trip checks for the DICOM decoders. Every decoder must
//! return a result for any input, and never panic. Dataset encoding must
//! survive a decode. Inputs come from a fixed seed.

use std::io::Cursor;

use tpt_t_medical_bridge_dicom::net::dataset::{
    Tag, decode_explicit, decode_implicit, encode_explicit, sequence_items,
};
use tpt_t_medical_bridge_dicom::net::pdu::{Pdu, read_pdu};
use tpt_t_medical_bridge_dicom::parse;

const SEEDS: u64 = 256;

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next_u64() % u64::try_from(n).unwrap_or(u64::MAX)).unwrap_or(0)
    }

    fn bytes(&mut self, max_len: usize) -> Vec<u8> {
        let len = self.below(max_len + 1);
        (0..len)
            .map(|_| u8::try_from(self.below(256)).unwrap_or(0))
            .collect()
    }
}

#[test]
fn random_bytes_never_panic_in_any_decoder() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(seed);
        let bytes = rng.bytes(700);
        let _ = decode_implicit(&bytes);
        let _ = decode_explicit(&bytes);
        let _ = sequence_items(&bytes);
        let _ = parse(&bytes);
        let _ = read_pdu(&mut Cursor::new(&bytes));
    }
}

#[test]
fn random_pdu_bodies_never_panic() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(seed);
        let kind = u8::try_from(rng.below(256)).unwrap_or(0);
        let body = rng.bytes(300);
        let _ = Pdu::decode(kind, &body);
    }
}

#[test]
fn every_truncation_of_an_encoded_dataset_is_handled() {
    let elements: Vec<(Tag, [u8; 2], Vec<u8>)> = vec![
        ((0x0010, 0x0010), *b"PN", b"DOE^JANE ".to_vec()),
        ((0x0010, 0x0020), *b"LO", b"MRN123  ".to_vec()),
        ((0x0028, 0x0010), *b"US", 512_u16.to_le_bytes().to_vec()),
    ];
    let bytes = encode_explicit(&elements);
    for len in 0..bytes.len() {
        let _ = decode_explicit(&bytes[..len]);
    }
    assert!(decode_explicit(&bytes).is_ok());
}

#[test]
fn explicit_dataset_round_trips() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(seed);
        let count = 1 + rng.below(8);
        let mut tags: Vec<Tag> = (0..count)
            .map(|_| {
                (
                    u16::try_from(rng.below(0x10000)).unwrap_or(0),
                    u16::try_from(rng.below(0x10000)).unwrap_or(0),
                )
            })
            .collect();
        tags.sort_unstable();
        tags.dedup();

        // Even-length text values, so no padding byte is added on the way.
        let elements: Vec<(Tag, [u8; 2], Vec<u8>)> = tags
            .iter()
            .map(|&tag| {
                let len = 2 * rng.below(16);
                let value: Vec<u8> = (0..len)
                    .map(|_| b'A' + u8::try_from(rng.below(26)).unwrap_or(0))
                    .collect();
                (tag, *b"LO", value)
            })
            .collect();

        let bytes = encode_explicit(&elements);
        let decoded = decode_explicit(&bytes).expect("own encoding decodes");
        assert_eq!(decoded.len(), elements.len(), "seed {seed}");
        for (got, (tag, vr, value)) in decoded.iter().zip(&elements) {
            assert_eq!(got.tag, *tag, "seed {seed}");
            assert_eq!(got.vr, Some(*vr), "seed {seed}");
            assert_eq!(&got.value, value, "seed {seed}");
        }
    }
}
