//! Consent tokens: signed, purpose-limited, expiring permission to use a
//! patient's data (spec.txt §3, §4.4).
//!
//! The token is signed by a consent authority. Anyone can read the token, but
//! only the authority can produce one. The signature covers the patient, the
//! purposes, and the expiry, so none of those can be changed without
//! invalidating it.

use std::fmt;

use sha2::{Digest, Sha256};
use tpt_t_medical_compliance::{Hash, Signer, Verifier};

/// Domain separator, so a consent signature cannot be replayed as an audit
/// signature.
const DOMAIN: &[u8] = b"tpt-consent-v1";

/// What the data may be used for. Each purpose is granted separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// Direct care of the patient.
    Treatment,
    /// Writing the data to logs.
    Logging,
    /// Including the data in a training dataset.
    Training,
}

impl Purpose {
    pub(crate) const fn bit(self) -> u8 {
        match self {
            Self::Treatment => 1 << 0,
            Self::Logging => 1 << 1,
            Self::Training => 1 << 2,
        }
    }
}

/// Why a consent check failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentError {
    /// The signature does not verify, so the token is not trusted.
    BadSignature,
    /// The token is for a different patient.
    WrongPatient,
    /// The token does not grant the requested purpose.
    NotConsented(Purpose),
    /// The token has expired.
    Expired,
    /// The token has been revoked by the consent authority.
    Revoked,
}

impl fmt::Display for ConsentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadSignature => f.write_str("consent token signature is invalid"),
            Self::WrongPatient => f.write_str("consent token is for a different patient"),
            Self::NotConsented(p) => write!(f, "consent not granted for {p:?}"),
            Self::Expired => f.write_str("consent token has expired"),
            Self::Revoked => f.write_str("consent token has been revoked"),
        }
    }
}

impl std::error::Error for ConsentError {}

/// A signed grant of consent for one patient.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsentToken {
    patient: [u8; 16],
    purposes: u8,
    expires_us: u64,
    signature: Vec<u8>,
}

impl ConsentToken {
    /// Returns the token's identifier: the digest of its signed fields. Used to
    /// revoke it.
    #[must_use]
    pub fn id(&self) -> Hash {
        digest(&self.patient, self.purposes, self.expires_us)
    }

    /// Issues a token. Called by the consent authority.
    ///
    /// The token is valid while `now_us < expires_us`.
    #[must_use]
    pub fn issue<G: Signer + ?Sized>(
        signer: &G,
        patient: [u8; 16],
        purposes: &[Purpose],
        expires_us: u64,
    ) -> Self {
        let purposes = purposes.iter().fold(0, |acc, p| acc | p.bit());
        let digest = digest(&patient, purposes, expires_us);
        Self {
            patient,
            purposes,
            expires_us,
            signature: signer.sign(&digest),
        }
    }

    /// Checks that the token allows `purpose` for `patient` at `now_us`.
    ///
    /// The signature is checked first, so no other field is trusted before it.
    ///
    /// # Errors
    ///
    /// Returns the first failed check.
    pub fn check<V: Verifier + ?Sized>(
        &self,
        verifier: &V,
        patient: [u8; 16],
        purpose: Purpose,
        now_us: u64,
    ) -> Result<(), ConsentError> {
        let digest = digest(&self.patient, self.purposes, self.expires_us);
        if !verifier.verify(&digest, &self.signature) {
            return Err(ConsentError::BadSignature);
        }
        if self.patient != patient {
            return Err(ConsentError::WrongPatient);
        }
        if self.purposes & purpose.bit() == 0 {
            return Err(ConsentError::NotConsented(purpose));
        }
        if now_us >= self.expires_us {
            return Err(ConsentError::Expired);
        }
        Ok(())
    }
}

fn digest(patient: &[u8; 16], purposes: u8, expires_us: u64) -> Hash {
    let mut hasher = Sha256::new();
    hasher.update(DOMAIN);
    hasher.update(patient);
    hasher.update([purposes]);
    hasher.update(expires_us.to_le_bytes());
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_t_medical_compliance::ed25519::Ed25519Signer;

    const PATIENT: [u8; 16] = [1; 16];
    const OTHER: [u8; 16] = [2; 16];

    fn authority() -> Ed25519Signer {
        Ed25519Signer::from_seed(&[11; 32])
    }

    fn token() -> ConsentToken {
        ConsentToken::issue(&authority(), PATIENT, &[Purpose::Treatment], 1_000)
    }

    #[test]
    fn granted_purpose_is_allowed_before_expiry() {
        let verifier = authority().verifier();
        assert_eq!(
            token().check(&verifier, PATIENT, Purpose::Treatment, 999),
            Ok(())
        );
    }

    #[test]
    fn expiry_is_exclusive() {
        let verifier = authority().verifier();
        assert_eq!(
            token().check(&verifier, PATIENT, Purpose::Treatment, 1_000),
            Err(ConsentError::Expired)
        );
    }

    #[test]
    fn ungranted_purpose_is_refused_even_if_others_are_granted() {
        let verifier = authority().verifier();
        assert_eq!(
            token().check(&verifier, PATIENT, Purpose::Training, 0),
            Err(ConsentError::NotConsented(Purpose::Training))
        );
    }

    #[test]
    fn token_for_another_patient_is_refused() {
        let verifier = authority().verifier();
        assert_eq!(
            token().check(&verifier, OTHER, Purpose::Treatment, 0),
            Err(ConsentError::WrongPatient)
        );
    }

    #[test]
    fn token_signed_by_another_authority_is_refused() {
        let impostor = Ed25519Signer::from_seed(&[12; 32]);
        let forged = ConsentToken::issue(&impostor, PATIENT, &[Purpose::Training], 1_000);
        let verifier = authority().verifier();
        assert_eq!(
            forged.check(&verifier, PATIENT, Purpose::Training, 0),
            Err(ConsentError::BadSignature)
        );
    }

    #[test]
    fn extending_the_expiry_invalidates_the_signature() {
        let verifier = authority().verifier();
        let mut extended = token();
        extended.expires_us = u64::MAX;
        assert_eq!(
            extended.check(&verifier, PATIENT, Purpose::Treatment, 5_000),
            Err(ConsentError::BadSignature)
        );
    }

    #[test]
    fn adding_a_purpose_invalidates_the_signature() {
        let verifier = authority().verifier();
        let mut widened = token();
        widened.purposes |= Purpose::Training.bit();
        assert_eq!(
            widened.check(&verifier, PATIENT, Purpose::Training, 0),
            Err(ConsentError::BadSignature)
        );
    }
}
