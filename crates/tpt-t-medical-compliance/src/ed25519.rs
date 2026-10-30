//! Ed25519 signatures for audit entries, via `ed25519-dalek` (BSD-3-Clause).
//!
//! The caller supplies the 32-byte secret seed. The spec asks for a key held
//! in a TPM or secure enclave (spec.txt §4.4). Hardware-backed keys are not
//! implemented yet, so this module only handles keys it is given.
//!
//! Verification uses `verify_strict`, which rejects non-canonical signatures
//! and small-order public keys. Without that, one signed entry could have more
//! than one valid signature, and the chain would be harder to reason about.

use ed25519_dalek::{Signature, Signer as _, SigningKey, VerifyingKey};

use crate::Hash;

/// Signs entry digests with an Ed25519 secret key.
pub struct Ed25519Signer {
    key: SigningKey,
}

impl Ed25519Signer {
    /// Creates a signer from a 32-byte secret seed.
    #[must_use]
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        Self {
            key: SigningKey::from_bytes(seed),
        }
    }

    /// Returns a verifier for this signer's public key.
    #[must_use]
    pub fn verifier(&self) -> Ed25519Verifier {
        Ed25519Verifier {
            key: self.key.verifying_key(),
        }
    }
}

impl crate::Signer for Ed25519Signer {
    fn sign(&self, hash: &Hash) -> Vec<u8> {
        let signature: Signature = self.key.sign(hash);
        signature.to_bytes().to_vec()
    }
}

/// Checks entry signatures against an Ed25519 public key.
pub struct Ed25519Verifier {
    key: VerifyingKey,
}

impl Ed25519Verifier {
    /// Creates a verifier from a 32-byte public key.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes are not a valid Ed25519 public key.
    pub fn from_public_key(public: &[u8; 32]) -> Result<Self, ed25519_dalek::SignatureError> {
        Ok(Self {
            key: VerifyingKey::from_bytes(public)?,
        })
    }
}

impl crate::Verifier for Ed25519Verifier {
    fn verify(&self, hash: &Hash, signature: &[u8]) -> bool {
        let Ok(signature) = Signature::from_slice(signature) else {
            return false;
        };
        self.key.verify_strict(hash, &signature).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChainErrorKind, Log};

    const SEED: [u8; 32] = [7; 32];

    fn signed_log(signer: &Ed25519Signer) -> Log {
        let mut log = Log::new();
        for payload in [&b"register"[..], b"activate", b"pause"] {
            log.append(signer, payload.to_vec());
        }
        log
    }

    #[test]
    fn signed_log_verifies_with_the_matching_public_key() {
        let signer = Ed25519Signer::from_seed(&SEED);
        let log = signed_log(&signer);
        assert_eq!(log.verify(&signer.verifier()), Ok(()));
    }

    #[test]
    fn signed_log_fails_under_a_different_public_key() {
        let signer = Ed25519Signer::from_seed(&SEED);
        let other = Ed25519Signer::from_seed(&[8; 32]);
        let log = signed_log(&signer);
        let err = log.verify(&other.verifier()).expect_err("wrong key");
        assert_eq!(err.index, 0);
        assert_eq!(err.kind, ChainErrorKind::Signature);
    }

    #[test]
    fn flipped_signature_bit_is_a_signature_failure_at_that_entry() {
        let signer = Ed25519Signer::from_seed(&SEED);
        let mut entries = signed_log(&signer).entries().to_vec();
        entries[1].signature[0] ^= 0x01;
        let err = crate::verify_entries(&entries, &signer.verifier()).expect_err("tampered");
        assert_eq!(err.index, 1);
        assert_eq!(err.kind, ChainErrorKind::Signature);
    }

    #[test]
    fn truncated_signature_is_rejected_without_panicking() {
        let signer = Ed25519Signer::from_seed(&SEED);
        let mut entries = signed_log(&signer).entries().to_vec();
        entries[0].signature.truncate(10);
        let err = crate::verify_entries(&entries, &signer.verifier()).expect_err("short");
        assert_eq!(err.kind, ChainErrorKind::Signature);
    }

    #[test]
    fn signing_is_deterministic_for_a_given_seed() {
        let a = Ed25519Signer::from_seed(&SEED);
        let b = Ed25519Signer::from_seed(&SEED);
        let hash = [3; 32];
        assert_eq!(
            crate::Signer::sign(&a, &hash),
            crate::Signer::sign(&b, &hash)
        );
    }

    #[test]
    fn public_key_bytes_round_trip_through_from_public_key() {
        let signer = Ed25519Signer::from_seed(&SEED);
        let public = signer.key.verifying_key().to_bytes();
        let verifier = Ed25519Verifier::from_public_key(&public).expect("valid key");
        let log = signed_log(&signer);
        assert_eq!(log.verify(&verifier), Ok(()));
    }
}
