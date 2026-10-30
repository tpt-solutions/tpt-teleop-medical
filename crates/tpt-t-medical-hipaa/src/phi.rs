//! PHI encryption with AES-256-GCM (spec.txt §4.4).
//!
//! Each record gets a fresh random 96-bit nonce from the operating system. The
//! sealed form is `nonce || ciphertext || tag`. The patient identifier is
//! passed as associated data, so a record sealed for one patient will not open
//! under another.
//!
//! Random 96-bit nonces are safe for up to about 2^32 records under one key.
//! Beyond that, rotate the key.

use std::fmt;

use aes_gcm::Aes256Gcm;
use aes_gcm::aead::{Aead, Key, KeyInit, Nonce, Payload};

/// Length of an AES-256 key in bytes.
pub const KEY_LEN: usize = 32;
/// Length of a GCM nonce in bytes.
pub const NONCE_LEN: usize = 12;
/// Length of a GCM authentication tag in bytes.
pub const TAG_LEN: usize = 16;

/// Why PHI could not be sealed or opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhiError {
    /// The operating system's random source failed.
    Rng,
    /// Encryption failed.
    Seal,
    /// The sealed bytes are too short to contain a nonce and tag.
    Truncated,
    /// Authentication failed. The bytes were altered, the wrong key was used,
    /// or the associated data does not match.
    Open,
}

impl fmt::Display for PhiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Rng => "random source unavailable",
            Self::Seal => "encryption failed",
            Self::Truncated => "sealed data is too short",
            Self::Open => "authentication failed",
        })
    }
}

impl std::error::Error for PhiError {}

/// An AES-256-GCM key for PHI.
///
/// The key is not `Clone`, and its `Debug` output is redacted. Keys come from
/// the caller for now. Hardware-backed storage is a later step.
pub struct PhiKey {
    cipher: Aes256Gcm,
}

impl PhiKey {
    /// Creates a key from 32 bytes.
    #[must_use]
    pub fn new(key: &[u8; KEY_LEN]) -> Self {
        Self {
            cipher: Aes256Gcm::new(&Key::<Aes256Gcm>::from(*key)),
        }
    }

    /// Encrypts `plaintext`, binding it to `aad`. Returns `nonce || ciphertext
    /// || tag`.
    ///
    /// # Errors
    ///
    /// Returns an error if the random source or encryption fails.
    pub fn seal(&self, aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, PhiError> {
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|_| PhiError::Rng)?;
        let ciphertext = self
            .cipher
            .encrypt(
                &Nonce::<Aes256Gcm>::from(nonce),
                Payload {
                    msg: plaintext,
                    aad,
                },
            )
            .map_err(|_| PhiError::Seal)?;

        let mut sealed = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        sealed.extend_from_slice(&nonce);
        sealed.extend_from_slice(&ciphertext);
        Ok(sealed)
    }

    /// Decrypts and authenticates `sealed` against `aad`.
    ///
    /// # Errors
    ///
    /// Returns an error if the data is too short or fails authentication.
    pub fn open(&self, aad: &[u8], sealed: &[u8]) -> Result<Phi, PhiError> {
        if sealed.len() < NONCE_LEN + TAG_LEN {
            return Err(PhiError::Truncated);
        }
        let (nonce, ciphertext) = sealed.split_at(NONCE_LEN);
        // `split_at` gives exactly NONCE_LEN bytes, so `from_slice` cannot panic.
        self.cipher
            .decrypt(
                Nonce::<Aes256Gcm>::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad,
                },
            )
            .map(Phi)
            .map_err(|_| PhiError::Open)
    }
}

impl fmt::Debug for PhiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PhiKey(<redacted>)")
    }
}

/// Decrypted PHI.
///
/// Read it with [`Phi::expose`]. `Debug` prints only the length, and there is
/// no `Display`, so the value cannot reach a log by accident.
pub struct Phi(Vec<u8>);

impl Phi {
    /// Returns the plaintext bytes.
    #[must_use]
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for Phi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Phi(<redacted {} bytes>)", self.0.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; KEY_LEN] = [0x42; KEY_LEN];
    const PATIENT: &[u8] = b"patient-0001";

    #[test]
    fn round_trips_with_matching_aad() {
        let key = PhiKey::new(&KEY);
        let sealed = key.seal(PATIENT, b"blood pressure 120/80").expect("seal");
        let opened = key.open(PATIENT, &sealed).expect("open");
        assert_eq!(opened.expose(), b"blood pressure 120/80");
    }

    #[test]
    fn same_plaintext_seals_differently_each_time() {
        let key = PhiKey::new(&KEY);
        let a = key.seal(PATIENT, b"same").expect("seal");
        let b = key.seal(PATIENT, b"same").expect("seal");
        assert_ne!(a, b, "nonces must not repeat");
    }

    #[test]
    fn wrong_patient_aad_fails_to_open() {
        let key = PhiKey::new(&KEY);
        let sealed = key.seal(PATIENT, b"record").expect("seal");
        assert_eq!(
            key.open(b"patient-0002", &sealed).expect_err("wrong aad"),
            PhiError::Open
        );
    }

    #[test]
    fn wrong_key_fails_to_open() {
        let sealed = PhiKey::new(&KEY).seal(PATIENT, b"record").expect("seal");
        let other = PhiKey::new(&[0x43; KEY_LEN]);
        assert_eq!(
            other.open(PATIENT, &sealed).expect_err("wrong key"),
            PhiError::Open
        );
    }

    #[test]
    fn every_flipped_bit_in_the_sealed_form_is_detected() {
        let key = PhiKey::new(&KEY);
        let sealed = key.seal(PATIENT, b"x").expect("seal");
        for i in 0..sealed.len() {
            let mut altered = sealed.clone();
            altered[i] ^= 0x80;
            assert!(
                key.open(PATIENT, &altered).is_err(),
                "byte {i} altered without detection"
            );
        }
    }

    #[test]
    fn data_shorter_than_nonce_plus_tag_is_truncated() {
        let key = PhiKey::new(&KEY);
        let short = vec![0u8; NONCE_LEN + TAG_LEN - 1];
        assert_eq!(
            key.open(PATIENT, &short).expect_err("short"),
            PhiError::Truncated
        );
    }

    #[test]
    fn debug_output_never_contains_plaintext_or_key_bytes() {
        let key = PhiKey::new(&KEY);
        let sealed = key.seal(PATIENT, b"secret-diagnosis").expect("seal");
        let phi = key.open(PATIENT, &sealed).expect("open");
        let shown = format!("{phi:?} {key:?}");
        assert!(!shown.contains("secret"), "{shown}");
        assert!(shown.contains("redacted"));
    }
}
