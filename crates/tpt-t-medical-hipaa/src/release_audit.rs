//! Audit records for every PHI access attempt (spec.txt §4.4).
//!
//! Each attempt is written to the compliance journal, granted or refused. A
//! granted attempt is recorded before the record is decrypted. If the journal
//! write fails, nothing is decrypted and no plaintext is returned.
//!
//! The record names the patient by their opaque identifier. It does not contain
//! any clinical content.

use std::fmt;

use tpt_t_medical_compliance::journal::{Journal, JournalError};
use tpt_t_medical_compliance::{Signer, Storage, Verifier};

use crate::consent::{ConsentError, ConsentToken};
use crate::phi::{Phi, PhiKey};
use crate::revocation::{RevocationSet, check_with_revocations};
use crate::{ReleaseError, Request};

const KIND_ACCESS: u8 = 1;
const OUTCOME_GRANTED: u8 = 0;

/// Why an audited release did not return plaintext.
#[derive(Debug)]
pub enum AuditedReleaseError {
    /// The access attempt could not be recorded. Nothing was decrypted.
    Audit(JournalError),
    /// The attempt was recorded, but it was refused or could not be decrypted.
    Release(ReleaseError),
}

impl fmt::Display for AuditedReleaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Audit(e) => write!(f, "access not recorded, nothing released: {e}"),
            Self::Release(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AuditedReleaseError {}

/// Stable outcome code for an access record. Do not renumber.
const fn outcome_code(error: ConsentError) -> u8 {
    match error {
        ConsentError::BadSignature => 1,
        ConsentError::WrongPatient => 2,
        ConsentError::NotConsented(_) => 3,
        ConsentError::Expired => 4,
        ConsentError::Revoked => 5,
    }
}

/// Journals PHI access attempts, then releases the data if consent allows.
pub struct ReleaseAudit<S: Storage, G: Signer> {
    journal: Journal<S>,
    signer: G,
}

impl<S: Storage, G: Signer> ReleaseAudit<S, G> {
    /// Opens the audit journal, verifying its existing records.
    ///
    /// # Errors
    ///
    /// Returns an error if the journal does not verify.
    pub fn open<V: Verifier + ?Sized>(
        store: S,
        verifier: &V,
        signer: G,
    ) -> Result<Self, JournalError> {
        Ok(Self {
            journal: Journal::open(store, verifier)?,
            signer,
        })
    }

    /// Returns the journal of access records.
    #[must_use]
    pub const fn journal(&self) -> &Journal<S> {
        &self.journal
    }

    /// Records the attempt, then releases the record if consent allows.
    ///
    /// # Errors
    ///
    /// Returns [`AuditedReleaseError::Audit`] if the attempt cannot be
    /// recorded, in which case nothing is decrypted. Returns
    /// [`AuditedReleaseError::Release`] if the attempt was refused or the
    /// record could not be opened.
    pub fn release<V: Verifier + ?Sized>(
        &mut self,
        key: &PhiKey,
        token: &ConsentToken,
        verifier: &V,
        revoked: &RevocationSet,
        request: &Request,
        sealed: &[u8],
    ) -> Result<Phi, AuditedReleaseError> {
        let consent = check_with_revocations(
            token,
            verifier,
            revoked,
            request.patient,
            request.purpose,
            request.now_us,
        );
        let outcome = match consent {
            Ok(()) => OUTCOME_GRANTED,
            Err(e) => outcome_code(e),
        };

        // Record before deciding anything else, so no plaintext can exist
        // without a record of it.
        self.journal
            .append(&self.signer, access_record(outcome, request))
            .map_err(AuditedReleaseError::Audit)?;

        consent.map_err(|e| AuditedReleaseError::Release(ReleaseError::Consent(e)))?;
        key.open(&request.patient, sealed)
            .map_err(|e| AuditedReleaseError::Release(ReleaseError::Phi(e)))
    }
}

fn access_record(outcome: u8, request: &Request) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + 1 + 1 + 16 + 8);
    out.push(KIND_ACCESS);
    out.push(outcome);
    out.push(request.purpose.bit());
    out.extend_from_slice(&request.patient);
    out.extend_from_slice(&request.now_us.to_le_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::consent::Purpose;
    use crate::phi::PhiError;
    use crate::seal_for_patient;
    use std::cell::Cell;
    use std::io;
    use std::rc::Rc;
    use tpt_t_medical_compliance::ed25519::Ed25519Signer;

    const SEED: [u8; 32] = [53; 32];
    const KEY: [u8; 32] = [0x66; 32];
    const PATIENT: [u8; 16] = [9; 16];

    /// Storage whose appends can be made to fail.
    struct Store {
        stored: Vec<Vec<u8>>,
        fail: Rc<Cell<bool>>,
    }

    impl Storage for Store {
        fn append(&mut self, record: &[u8]) -> io::Result<()> {
            if self.fail.get() {
                return Err(io::Error::other("storage unavailable"));
            }
            self.stored.push(record.to_vec());
            Ok(())
        }

        fn records(&mut self) -> io::Result<Vec<Vec<u8>>> {
            Ok(self.stored.clone())
        }
    }

    fn authority() -> Ed25519Signer {
        Ed25519Signer::from_seed(&SEED)
    }

    fn audit(fail: &Rc<Cell<bool>>) -> ReleaseAudit<Store, Ed25519Signer> {
        let store = Store {
            stored: Vec::new(),
            fail: Rc::clone(fail),
        };
        ReleaseAudit::open(store, &authority().verifier(), authority()).expect("open")
    }

    fn request(purpose: Purpose) -> Request {
        Request {
            patient: PATIENT,
            purpose,
            now_us: 5,
        }
    }

    #[test]
    fn granted_release_is_recorded_and_returns_plaintext() {
        let fail = Rc::new(Cell::new(false));
        let mut a = audit(&fail);
        let key = PhiKey::new(&KEY);
        let sealed = seal_for_patient(&key, PATIENT, b"dose: 5 mg").expect("seal");
        let token = ConsentToken::issue(&authority(), PATIENT, &[Purpose::Treatment], 100);

        let phi = a
            .release(
                &key,
                &token,
                &authority().verifier(),
                &RevocationSet::new(),
                &request(Purpose::Treatment),
                &sealed,
            )
            .expect("released");

        assert_eq!(phi.expose(), b"dose: 5 mg");
        let entries = a.journal().log().entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].payload[1], OUTCOME_GRANTED);
    }

    #[test]
    fn refused_attempt_is_recorded_with_its_reason() {
        let fail = Rc::new(Cell::new(false));
        let mut a = audit(&fail);
        let key = PhiKey::new(&KEY);
        let sealed = seal_for_patient(&key, PATIENT, b"record").expect("seal");
        let token = ConsentToken::issue(&authority(), PATIENT, &[Purpose::Treatment], 100);

        let err = a
            .release(
                &key,
                &token,
                &authority().verifier(),
                &RevocationSet::new(),
                &request(Purpose::Training),
                &sealed,
            )
            .expect_err("training not consented");

        assert!(matches!(
            err,
            AuditedReleaseError::Release(ReleaseError::Consent(ConsentError::NotConsented(
                Purpose::Training
            )))
        ));
        let entry = &a.journal().log().entries()[0];
        assert_eq!(
            entry.payload[1],
            outcome_code(ConsentError::NotConsented(Purpose::Training))
        );
    }

    #[test]
    fn revoked_token_is_recorded_as_revoked() {
        let fail = Rc::new(Cell::new(false));
        let mut a = audit(&fail);
        let key = PhiKey::new(&KEY);
        let sealed = seal_for_patient(&key, PATIENT, b"record").expect("seal");
        let token = ConsentToken::issue(&authority(), PATIENT, &[Purpose::Treatment], 100);
        let mut revoked = RevocationSet::new();
        revoked.insert(token.id());

        let err = a
            .release(
                &key,
                &token,
                &authority().verifier(),
                &revoked,
                &request(Purpose::Treatment),
                &sealed,
            )
            .expect_err("revoked");

        assert!(matches!(
            err,
            AuditedReleaseError::Release(ReleaseError::Consent(ConsentError::Revoked))
        ));
        assert_eq!(a.journal().log().entries()[0].payload[1], 5);
    }

    #[test]
    fn failed_audit_write_releases_nothing() {
        let fail = Rc::new(Cell::new(true));
        let mut a = audit(&fail);
        let key = PhiKey::new(&KEY);
        let sealed = seal_for_patient(&key, PATIENT, b"record").expect("seal");
        let token = ConsentToken::issue(&authority(), PATIENT, &[Purpose::Treatment], 100);

        let err = a
            .release(
                &key,
                &token,
                &authority().verifier(),
                &RevocationSet::new(),
                &request(Purpose::Treatment),
                &sealed,
            )
            .expect_err("audit failed");

        assert!(matches!(err, AuditedReleaseError::Audit(_)));
    }

    #[test]
    fn wrong_patient_record_is_recorded_and_fails_to_open() {
        let fail = Rc::new(Cell::new(false));
        let mut a = audit(&fail);
        let key = PhiKey::new(&KEY);
        let other_patients = seal_for_patient(&key, [1; 16], b"record").expect("seal");
        let token = ConsentToken::issue(&authority(), PATIENT, &[Purpose::Treatment], 100);

        let err = a
            .release(
                &key,
                &token,
                &authority().verifier(),
                &RevocationSet::new(),
                &request(Purpose::Treatment),
                &other_patients,
            )
            .expect_err("bound to another patient");

        assert!(matches!(
            err,
            AuditedReleaseError::Release(ReleaseError::Phi(PhiError::Open))
        ));
        assert_eq!(
            a.journal().log().entries().len(),
            1,
            "the attempt is still recorded"
        );
    }
}
