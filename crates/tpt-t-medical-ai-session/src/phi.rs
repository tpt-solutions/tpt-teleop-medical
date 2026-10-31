//! PHI gate for the AI controller (docs/design/ai-session.md §1.6).
//!
//! The AI sees patient data only through [`tpt_t_medical_hipaa::release`], which
//! checks the consent token, checks its revocation, and only then decrypts. This
//! module adds one rule of its own: the AI may read for treatment, or for a
//! training dataset that has its own consent. It may not read for logging. The
//! audit journal must hold no PHI, so PHI is never read to be logged.

use tpt_t_medical_compliance::Verifier;
use tpt_t_medical_hipaa::{
    ConsentToken, Phi, PhiKey, Purpose, ReleaseError, Request, RevocationSet, release,
};

/// Why the AI could not read a record.
#[derive(Debug)]
pub enum PhiGateError {
    /// The AI may not read PHI for logging.
    LoggingRefused,
    /// The HIPAA release refused: consent, revocation, or the record itself.
    Release(ReleaseError),
}

impl std::fmt::Display for PhiGateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LoggingRefused => f.write_str("the AI may not read PHI for logging"),
            Self::Release(e) => write!(f, "{e:?}"),
        }
    }
}

impl std::error::Error for PhiGateError {}

/// Reads one sealed record for the AI, if the request is for treatment or
/// training and consent allows it.
///
/// # Errors
///
/// Returns [`PhiGateError::LoggingRefused`] for a logging request, without
/// touching the key. Otherwise returns the HIPAA release error.
pub fn read_for_ai<V: Verifier + ?Sized>(
    key: &PhiKey,
    token: &ConsentToken,
    verifier: &V,
    revoked: &RevocationSet,
    request: &Request,
    sealed: &[u8],
) -> Result<Phi, PhiGateError> {
    if request.purpose == Purpose::Logging {
        return Err(PhiGateError::LoggingRefused);
    }
    release(key, token, verifier, revoked, request, sealed).map_err(PhiGateError::Release)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use tpt_t_medical_compliance::ed25519::Ed25519Signer;
    use tpt_t_medical_hipaa::seal_for_patient;

    const KEY: [u8; 32] = [0x21; 32];
    const PATIENT: [u8; 16] = [0xA1; 16];
    const OTHER: [u8; 16] = [0xB0; 16];
    const NOW: u64 = 1_000;
    const LATER: u64 = 10_000;

    fn authority() -> Ed25519Signer {
        Ed25519Signer::from_seed(&[31; 32])
    }

    fn request(patient: [u8; 16], purpose: Purpose) -> Request {
        Request {
            patient,
            purpose,
            now_us: NOW,
        }
    }

    fn sealed(key: &PhiKey) -> Vec<u8> {
        seal_for_patient(key, PATIENT, b"allergy: penicillin").expect("seal")
    }

    #[test]
    fn treatment_with_consent_is_read() {
        let key = PhiKey::new(&KEY);
        let auth = authority();
        let token = ConsentToken::issue(&auth, PATIENT, &[Purpose::Treatment], LATER);
        let phi = read_for_ai(
            &key,
            &token,
            &auth.verifier(),
            &BTreeSet::new(),
            &request(PATIENT, Purpose::Treatment),
            &sealed(&key),
        )
        .expect("read");
        assert_eq!(phi.expose(), b"allergy: penicillin");
    }

    #[test]
    fn training_needs_its_own_consent() {
        let key = PhiKey::new(&KEY);
        let auth = authority();
        let treatment_only = ConsentToken::issue(&auth, PATIENT, &[Purpose::Treatment], LATER);
        let refused = read_for_ai(
            &key,
            &treatment_only,
            &auth.verifier(),
            &BTreeSet::new(),
            &request(PATIENT, Purpose::Training),
            &sealed(&key),
        );
        assert!(matches!(refused, Err(PhiGateError::Release(_))));
    }

    #[test]
    fn logging_is_refused_even_with_consent() {
        let key = PhiKey::new(&KEY);
        let auth = authority();
        let everything = ConsentToken::issue(
            &auth,
            PATIENT,
            &[Purpose::Treatment, Purpose::Logging, Purpose::Training],
            LATER,
        );
        let refused = read_for_ai(
            &key,
            &everything,
            &auth.verifier(),
            &BTreeSet::new(),
            &request(PATIENT, Purpose::Logging),
            &sealed(&key),
        );
        assert!(matches!(refused, Err(PhiGateError::LoggingRefused)));
    }

    #[test]
    fn a_revoked_token_is_refused() {
        let key = PhiKey::new(&KEY);
        let auth = authority();
        let token = ConsentToken::issue(&auth, PATIENT, &[Purpose::Treatment], LATER);
        let mut revoked = BTreeSet::new();
        revoked.insert(token.id());
        let refused = read_for_ai(
            &key,
            &token,
            &auth.verifier(),
            &revoked,
            &request(PATIENT, Purpose::Treatment),
            &sealed(&key),
        );
        assert!(matches!(
            refused,
            Err(PhiGateError::Release(ReleaseError::Consent(_)))
        ));
    }

    #[test]
    fn a_token_for_another_patient_is_refused() {
        let key = PhiKey::new(&KEY);
        let auth = authority();
        let token = ConsentToken::issue(&auth, OTHER, &[Purpose::Treatment], LATER);
        let refused = read_for_ai(
            &key,
            &token,
            &auth.verifier(),
            &BTreeSet::new(),
            &request(PATIENT, Purpose::Treatment),
            &sealed(&key),
        );
        assert!(matches!(refused, Err(PhiGateError::Release(_))));
    }
}
