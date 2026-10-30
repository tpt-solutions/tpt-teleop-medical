//! HIPAA-oriented data handling for tpt-teleop-medical (spec.txt §3, §4.4).
//!
//! Two parts:
//!
//! * [`phi`] encrypts patient data with AES-256-GCM. Each record is bound to
//!   its patient identifier.
//! * [`consent`] issues and checks signed consent tokens, which grant a
//!   patient's data for specific purposes until an expiry time.
//!
//! [`release`] is the only way to read PHI back. It checks consent first and
//! decrypts only if consent passes.
//!
//! Not implemented yet: key storage in hardware, consent revocation, and
//! audit records for each release. A release should be journalled through
//! `tpt-t-medical-compliance`, but that wiring is left to the caller.

#![forbid(unsafe_code)]

pub mod consent;
pub mod phi;

use core::fmt;

use tpt_t_medical_compliance::Verifier;

pub use consent::{ConsentError, ConsentToken, Purpose};
pub use phi::{Phi, PhiError, PhiKey};

/// Why PHI was not released.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseError {
    /// Consent did not allow this use.
    Consent(ConsentError),
    /// Consent passed, but the record could not be decrypted.
    Phi(PhiError),
}

impl fmt::Display for ReleaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Consent(e) => write!(f, "release refused: {e}"),
            Self::Phi(e) => write!(f, "release failed: {e}"),
        }
    }
}

impl std::error::Error for ReleaseError {}

/// Seals `plaintext` for `patient`. The patient identifier is the associated
/// data, so the record can only be opened for that patient.
///
/// # Errors
///
/// Returns an error if encryption fails.
pub fn seal_for_patient(
    key: &PhiKey,
    patient: [u8; 16],
    plaintext: &[u8],
) -> Result<Vec<u8>, PhiError> {
    key.seal(&patient, plaintext)
}

/// Decrypts a record for `patient`, but only if `token` grants `purpose` at
/// `now_us`.
///
/// Consent is checked before any decryption, so a refused request never
/// touches the key.
///
/// # Errors
///
/// Returns [`ReleaseError::Consent`] if consent does not allow the use, or
/// [`ReleaseError::Phi`] if the record cannot be opened for this patient.
pub fn release<V: Verifier + ?Sized>(
    key: &PhiKey,
    token: &ConsentToken,
    verifier: &V,
    patient: [u8; 16],
    purpose: Purpose,
    now_us: u64,
    sealed: &[u8],
) -> Result<Phi, ReleaseError> {
    token
        .check(verifier, patient, purpose, now_us)
        .map_err(ReleaseError::Consent)?;
    key.open(&patient, sealed).map_err(ReleaseError::Phi)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_t_medical_compliance::ed25519::Ed25519Signer;

    const KEY: [u8; 32] = [0x21; 32];
    const ALICE: [u8; 16] = [0xA1; 16];
    const BOB: [u8; 16] = [0xB0; 16];

    fn authority() -> Ed25519Signer {
        Ed25519Signer::from_seed(&[31; 32])
    }

    #[test]
    fn release_returns_plaintext_when_consent_passes() {
        let key = PhiKey::new(&KEY);
        let auth = authority();
        let sealed = seal_for_patient(&key, ALICE, b"allergy: penicillin").expect("seal");
        let token = ConsentToken::issue(&auth, ALICE, &[Purpose::Treatment], 10_000);

        let phi = release(
            &key,
            &token,
            &auth.verifier(),
            ALICE,
            Purpose::Treatment,
            1,
            &sealed,
        )
        .expect("released");

        assert_eq!(phi.expose(), b"allergy: penicillin");
    }

    #[test]
    fn missing_consent_refuses_without_decrypting() {
        let key = PhiKey::new(&KEY);
        let auth = authority();
        let sealed = seal_for_patient(&key, ALICE, b"record").expect("seal");
        let token = ConsentToken::issue(&auth, ALICE, &[Purpose::Treatment], 10_000);

        let err = release(
            &key,
            &token,
            &auth.verifier(),
            ALICE,
            Purpose::Training,
            1,
            &sealed,
        )
        .expect_err("training not consented");

        assert_eq!(
            err,
            ReleaseError::Consent(ConsentError::NotConsented(Purpose::Training))
        );
    }

    #[test]
    fn valid_consent_for_one_patient_cannot_open_another_patients_record() {
        // Bob's consent is genuine, but the record is Alice's. The consent check
        // passes, and decryption fails because the associated data is Alice's.
        let key = PhiKey::new(&KEY);
        let auth = authority();
        let alices_record = seal_for_patient(&key, ALICE, b"alice's record").expect("seal");
        let bobs_consent = ConsentToken::issue(&auth, BOB, &[Purpose::Treatment], 10_000);

        let err = release(
            &key,
            &bobs_consent,
            &auth.verifier(),
            BOB,
            Purpose::Treatment,
            1,
            &alices_record,
        )
        .expect_err("record bound to alice");

        assert_eq!(err, ReleaseError::Phi(PhiError::Open));
    }

    #[test]
    fn expired_consent_refuses_release() {
        let key = PhiKey::new(&KEY);
        let auth = authority();
        let sealed = seal_for_patient(&key, ALICE, b"record").expect("seal");
        let token = ConsentToken::issue(&auth, ALICE, &[Purpose::Treatment], 100);

        let err = release(
            &key,
            &token,
            &auth.verifier(),
            ALICE,
            Purpose::Treatment,
            100,
            &sealed,
        )
        .expect_err("expired");

        assert_eq!(err, ReleaseError::Consent(ConsentError::Expired));
    }
}
