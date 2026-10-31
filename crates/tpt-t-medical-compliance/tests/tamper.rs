//! Property checks for tamper evidence (spec.txt §4.4).
//!
//! No external generator is used, so the cases are reproducible. Each seed
//! drives a small xorshift PRNG that picks log lengths, payloads and edits.

use tpt_t_medical_compliance::ed25519::Ed25519Signer;
use tpt_t_medical_compliance::{ChainErrorKind, Entry, Log, Signer, digest, verify_entries};

const SEEDS: u64 = 64;

/// Minimal xorshift64 generator. Never seeded with zero.
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

    /// Uniform-ish value in `0..n`. `n` must be non-zero.
    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next_u64() % u64::try_from(n).unwrap_or(u64::MAX))
            .unwrap_or(usize::MAX)
    }

    fn payload(&mut self) -> Vec<u8> {
        let len = self.below(48);
        (0..len)
            .map(|_| u8::try_from(self.below(256)).unwrap_or(0))
            .collect()
    }
}

fn random_log(rng: &mut Rng, signer: &Ed25519Signer) -> Log {
    let mut log = Log::new();
    let len = 2 + rng.below(12);
    for _ in 0..len {
        let payload = rng.payload();
        log.append(signer, payload);
    }
    log
}

#[test]
fn untouched_random_logs_verify() {
    let signer = Ed25519Signer::from_seed(&[7; 32]);
    let verifier = signer.verifier();
    for seed in 0..SEEDS {
        let mut rng = Rng::new(seed);
        let log = random_log(&mut rng, &signer);
        assert_eq!(log.verify(&verifier), Ok(()), "seed {seed}");
    }
}

#[test]
fn changing_any_payload_fails_at_that_entry() {
    let signer = Ed25519Signer::from_seed(&[7; 32]);
    let verifier = signer.verifier();
    for seed in 0..SEEDS {
        let mut rng = Rng::new(seed);
        let log = random_log(&mut rng, &signer);
        let original = log.entries().to_vec();
        let target = rng.below(original.len());

        let mut entries = original.clone();
        if entries[target].payload.is_empty() {
            entries[target].payload.push(0x5A);
        } else {
            let at = rng.below(entries[target].payload.len());
            entries[target].payload[at] ^= 1 << rng.below(8);
        }

        let err = verify_entries(&entries, &verifier).expect_err("tamper must be caught");
        assert_eq!(err.index, target, "seed {seed}");
        assert_eq!(err.kind, ChainErrorKind::Digest, "seed {seed}");
    }
}

#[test]
fn rewriting_an_entry_with_fresh_digest_and_signature_breaks_the_next_link() {
    // An attacker who recomputes the edited entry's digest and re-signs it
    // with the real key still breaks the next entry's link to it.
    let signer = Ed25519Signer::from_seed(&[7; 32]);
    let verifier = signer.verifier();
    for seed in 0..SEEDS {
        let mut rng = Rng::new(seed);
        let log = random_log(&mut rng, &signer);
        if log.entries().len() < 2 {
            continue;
        }
        let mut entries = log.entries().to_vec();
        let target = rng.below(entries.len() - 1);
        let new_payload = rng.payload();
        entries[target].payload = new_payload;
        entries[target].hash = digest(
            entries[target].seq,
            &entries[target].prev,
            &entries[target].payload,
        );
        entries[target].signature = signer.sign(&entries[target].hash);

        let err = verify_entries(&entries, &verifier).expect_err("insider rewrite must be caught");
        assert_eq!(err.index, target + 1, "seed {seed}");
        assert_eq!(err.kind, ChainErrorKind::Link, "seed {seed}");
    }
}

#[test]
fn removing_any_entry_is_a_sequence_failure() {
    let signer = Ed25519Signer::from_seed(&[7; 32]);
    let verifier = signer.verifier();
    for seed in 0..SEEDS {
        let mut rng = Rng::new(seed);
        let log = random_log(&mut rng, &signer);
        let mut entries = log.entries().to_vec();
        // Removing the tail is a valid shorter chain; see the truncation test.
        let target = rng.below(entries.len() - 1);
        entries.remove(target);

        let err = verify_entries(&entries, &verifier).expect_err("gap must be caught");
        assert_eq!(err.index, target, "seed {seed}");
        assert_eq!(err.kind, ChainErrorKind::Sequence, "seed {seed}");
    }
}

#[test]
fn swapping_adjacent_entries_is_a_sequence_failure() {
    let signer = Ed25519Signer::from_seed(&[7; 32]);
    let verifier = signer.verifier();
    for seed in 0..SEEDS {
        let mut rng = Rng::new(seed);
        let log = random_log(&mut rng, &signer);
        let mut entries = log.entries().to_vec();
        let at = rng.below(entries.len() - 1);
        entries.swap(at, at + 1);

        let err = verify_entries(&entries, &verifier).expect_err("reorder must be caught");
        assert_eq!(err.index, at, "seed {seed}");
        assert_eq!(err.kind, ChainErrorKind::Sequence, "seed {seed}");
    }
}

#[test]
fn truncating_the_tail_still_verifies_but_truncating_the_head_does_not() {
    // Dropping newest entries leaves a valid shorter chain. Callers that need
    // completeness must pin the head hash externally, as the seal flow does.
    // Dropping the oldest entry must fail.
    let signer = Ed25519Signer::from_seed(&[7; 32]);
    let verifier = signer.verifier();
    for seed in 0..SEEDS {
        let mut rng = Rng::new(seed);
        let log = random_log(&mut rng, &signer);
        let entries = log.entries().to_vec();
        let keep = 1 + rng.below(entries.len() - 1);
        assert_eq!(
            verify_entries(&entries[..keep], &verifier),
            Ok(()),
            "seed {seed}"
        );

        let err = verify_entries(&entries[1..], &verifier).expect_err("head drop must be caught");
        assert_eq!(err.index, 0, "seed {seed}");
        assert_eq!(err.kind, ChainErrorKind::Sequence, "seed {seed}");
    }
}

#[test]
fn every_field_of_the_first_entry_is_covered() {
    // Flip one bit in each field of entry 0 and require the chain to fail.
    let signer = Ed25519Signer::from_seed(&[7; 32]);
    let verifier = signer.verifier();
    let mut rng = Rng::new(1);
    let log = random_log(&mut rng, &signer);
    let base: Entry = log.entries()[0].clone();

    let mut cases: Vec<(&str, Entry)> = Vec::new();
    let mut e = base.clone();
    e.seq ^= 1;
    cases.push(("seq", e));
    let mut e = base.clone();
    e.prev[0] ^= 1;
    cases.push(("prev", e));
    let mut e = base.clone();
    e.hash[31] ^= 0x80;
    cases.push(("hash", e));
    let mut e = base.clone();
    e.signature[0] ^= 1;
    cases.push(("signature", e));
    let mut e = base.clone();
    e.payload.push(0);
    cases.push(("payload", e));

    for (field, edited) in cases {
        let mut entries = log.entries().to_vec();
        entries[0] = edited;
        assert!(
            verify_entries(&entries, &verifier).is_err(),
            "flip in {field} was accepted"
        );
    }
}

#[test]
fn signatures_from_another_key_are_rejected() {
    let signer = Ed25519Signer::from_seed(&[7; 32]);
    let other = Ed25519Signer::from_seed(&[8; 32]);
    for seed in 0..SEEDS {
        let mut rng = Rng::new(seed);
        let log = random_log(&mut rng, &signer);
        let err = log
            .verify(&other.verifier())
            .expect_err("foreign key must be rejected");
        assert_eq!(err.index, 0, "seed {seed}");
        assert_eq!(err.kind, ChainErrorKind::Signature, "seed {seed}");
    }
}
