//! Robustness checks for the HL7 v2 ADT parser. The parser must return a
//! result for any input and never panic. These tests feed it random bytes and
//! mutated valid messages, all from a fixed seed.

use tpt_t_medical_bridge_hl7fhir::parse;

const SEEDS: u64 = 256;

const VALID: &[u8] =
    b"MSH|^~\\&|HIS|HOSP|LIS|HOSP|202601011200||ADT^A01|MSG001|P|2.5\rPID|1||MRN123^^^HOSP||DOE^JANE\rPV1|1|I|WARD^101^A";

/// Characters that appear in HL7 messages. Random picks from this set reach
/// the segment and field logic far more often than uniform bytes do.
const ALPHABET: &[u8] = b"MSHPIDV1|^~\\&\r\n ADTA0123456789abcMRNWARD";

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

    fn byte(&mut self) -> u8 {
        u8::try_from(self.below(256)).unwrap_or(0)
    }
}

#[test]
fn random_bytes_never_panic() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(seed);
        let len = rng.below(600);
        let bytes: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
        let _ = parse(&bytes);
    }
}

#[test]
fn random_structured_text_never_panics() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(seed);
        let len = rng.below(600);
        let bytes: Vec<u8> = (0..len)
            .map(|_| ALPHABET[rng.below(ALPHABET.len())])
            .collect();
        let _ = parse(&bytes);
    }
}

#[test]
fn every_truncation_of_a_valid_message_never_panics() {
    for len in 0..=VALID.len() {
        let _ = parse(&VALID[..len]);
    }
}

#[test]
fn mutated_valid_messages_never_panic() {
    for seed in 0..SEEDS {
        let mut rng = Rng::new(seed);
        let mut bytes = VALID.to_vec();
        for _ in 0..(1 + rng.below(8)) {
            let at = rng.below(bytes.len());
            match rng.below(3) {
                0 => bytes[at] = rng.byte(),
                1 => bytes[at] = ALPHABET[rng.below(ALPHABET.len())],
                _ => {
                    bytes.remove(at);
                    if bytes.is_empty() {
                        bytes.push(b'M');
                    }
                }
            }
        }
        let _ = parse(&bytes);
    }
}

#[test]
fn the_unmutated_message_still_parses() {
    assert!(parse(VALID).is_ok());
}
