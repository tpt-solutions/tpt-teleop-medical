//! HIPAA-oriented data handling for tpt-teleop-medical (spec.txt §3, §4.4).
//!
//! * [`phi`] encrypts patient data with AES-256-GCM. Each record is bound to
//!   its patient identifier.
//! * [`consent`] issues and checks signed consent tokens, which grant a
//!   patient's data for specific purposes until an expiry time.
//! * [`revocation`] journals consent revocations, so a revoked token stays
//!   revoked across restarts.
//! * [`release_audit`] journals every access attempt, granted or refused,
//!   before any plaintext is returned.
//!
//! [`release`] is the unaudited building block. It checks consent first and
//! decrypts only if consent passes. Production code should use
//! [`release_audit::ReleaseAudit`], which records the attempt as well.
//!
//! Not implemented yet: key storage in hardware.

#![forbid(unsafe_code)]

pub mod consent;
pub mod phi;
pub mod release_audit;
pub mod revocation;

use core::fmt;

use tpt_t_medical_compliance::Verifier;

pub use consent::{ConsentError, ConsentToken, Purpose};
pub use phi::{Phi, PhiError, PhiKey};
pub use release_audit::{AuditedReleaseError, ReleaseAudit};
pub use revocation::{RevocationError, RevocationLedger, RevocationSet};

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

/// One request to read a patient's PHI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    /// The patient whose record is wanted.
    pub patient: [u8; 16],
    /// The use the data is wanted for.
    pub purpose: Purpose,
    /// The time of the request, in microseconds.
    pub now_us: u64,
}

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

/// Decrypts a record for `request.patient`, but only if `token` grants
/// `request.purpose` and has not been revoked.
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
    revoked: &RevocationSet,
    request: &Request,
    sealed: &[u8],
) -> Result<Phi, ReleaseError> {
    revocation::check_with_revocations(
        token,
        verifier,
        revoked,
        request.patient,
        request.purpose,
        request.now_us,
    )
    .map_err(ReleaseError::Consent)?;
    key.open(&request.patient, sealed)
        .map_err(ReleaseError::Phi)
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

    fn request(patient: [u8; 16], purpose: Purpose, now_us: u64) -> Request {
        Request {
            patient,
            purpose,
            now_us,
        }
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
            &RevocationSet::new(),
            &request(ALICE, Purpose::Treatment, 1),
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
            &RevocationSet::new(),
            &request(ALICE, Purpose::Training, 1),
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
            &RevocationSet::new(),
            &request(BOB, Purpose::Treatment, 1),
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
            &RevocationSet::new(),
            &request(ALICE, Purpose::Treatment, 100),
            &sealed,
        )
        .expect_err("expired");

        assert_eq!(err, ReleaseError::Consent(ConsentError::Expired));
    }

    #[test]
    fn revoked_consent_refuses_release() {
        let key = PhiKey::new(&KEY);
        let auth = authority();
        let sealed = seal_for_patient(&key, ALICE, b"record").expect("seal");
        let token = ConsentToken::issue(&auth, ALICE, &[Purpose::Treatment], 10_000);
        let mut revoked = RevocationSet::new();
        revoked.insert(token.id());

        let err = release(
            &key,
            &token,
            &auth.verifier(),
            &revoked,
            &request(ALICE, Purpose::Treatment, 1),
            &sealed,
        )
        .expect_err("revoked");

        assert_eq!(err, ReleaseError::Consent(ConsentError::Revoked));
    }
}
