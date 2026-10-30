//! Consent revocation (spec.txt §4.4).
//!
//! The consent authority can withdraw a token before it expires. Each
//! revocation is written to the compliance journal, signed and hash-chained,
//! and the revoked set is rebuilt by replaying the journal on open. A revoked
//! token stays revoked after a restart, and deleting a revocation record breaks
//! the chain.

use std::collections::BTreeSet;
use std::fmt;

use tpt_t_medical_compliance::journal::{Journal, JournalError};
use tpt_t_medical_compliance::{Hash, Signer, Storage, Verifier};

use crate::consent::{ConsentError, ConsentToken, Purpose};

/// The identifiers of revoked tokens.
pub type RevocationSet = BTreeSet<Hash>;

/// Like [`ConsentToken::check`], but also refuses a revoked token.
///
/// # Errors
///
/// Returns the first failed check. The revocation check comes last, after the
/// signature has been verified.
pub fn check_with_revocations<V: Verifier + ?Sized>(
    token: &ConsentToken,
    verifier: &V,
    revoked: &RevocationSet,
    patient: [u8; 16],
    purpose: Purpose,
    now_us: u64,
) -> Result<(), ConsentError> {
    token.check(verifier, patient, purpose, now_us)?;
    if revoked.contains(&token.id()) {
        return Err(ConsentError::Revoked);
    }
    Ok(())
}

/// Why a revocation operation failed.
#[derive(Debug)]
pub enum RevocationError {
    /// The journal could not be written or verified.
    Journal(JournalError),
    /// A stored revocation record is malformed.
    Malformed {
        /// Zero-based index of the record.
        record: usize,
    },
}

impl fmt::Display for RevocationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Journal(e) => write!(f, "revocation journal: {e}"),
            Self::Malformed { record } => write!(f, "revocation record {record} is malformed"),
        }
    }
}

impl std::error::Error for RevocationError {}

impl From<JournalError> for RevocationError {
    fn from(e: JournalError) -> Self {
        Self::Journal(e)
    }
}

const KIND_REVOKE: u8 = 1;
const RECORD_LEN: usize = 1 + 32 + 8;

/// The journalled set of revoked tokens.
pub struct RevocationLedger<S: Storage, G: Signer> {
    journal: Journal<S>,
    signer: G,
    revoked: RevocationSet,
}

impl<S: Storage, G: Signer> fmt::Debug for RevocationLedger<S, G> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RevocationLedger")
            .field("revoked", &self.revoked.len())
            .finish_non_exhaustive()
    }
}

impl<S: Storage, G: Signer> RevocationLedger<S, G> {
    /// Opens the ledger, verifying the journal and rebuilding the revoked set.
    ///
    /// # Errors
    ///
    /// Returns an error if the journal does not verify or a record is
    /// malformed.
    pub fn open<V: Verifier + ?Sized>(
        store: S,
        verifier: &V,
        signer: G,
    ) -> Result<Self, RevocationError> {
        let journal = Journal::open(store, verifier)?;
        let mut revoked = RevocationSet::new();
        for (index, entry) in journal.log().entries().iter().enumerate() {
            let id = parse(&entry.payload).ok_or(RevocationError::Malformed { record: index })?;
            revoked.insert(id);
        }
        Ok(Self {
            journal,
            signer,
            revoked,
        })
    }

    /// Returns the revoked identifiers, for use with
    /// [`check_with_revocations`].
    #[must_use]
    pub const fn revoked(&self) -> &RevocationSet {
        &self.revoked
    }

    /// Revokes `token`. Revoking an already-revoked token is a no-op and writes
    /// nothing.
    ///
    /// # Errors
    ///
    /// Returns an error if the revocation cannot be written. The token is not
    /// treated as revoked in that case.
    pub fn revoke(&mut self, token: &ConsentToken, now_us: u64) -> Result<(), RevocationError> {
        let id = token.id();
        if self.revoked.contains(&id) {
            return Ok(());
        }
        let mut payload = Vec::with_capacity(RECORD_LEN);
        payload.push(KIND_REVOKE);
        payload.extend_from_slice(&id);
        payload.extend_from_slice(&now_us.to_le_bytes());
        self.journal.append(&self.signer, payload)?;
        self.revoked.insert(id);
        Ok(())
    }
}

fn parse(payload: &[u8]) -> Option<Hash> {
    if payload.len() != RECORD_LEN || payload[0] != KIND_REVOKE {
        return None;
    }
    payload[1..33].try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io;
    use std::rc::Rc;
    use tpt_t_medical_compliance::ed25519::Ed25519Signer;

    const SEED: [u8; 32] = [41; 32];
    const PATIENT: [u8; 16] = [7; 16];

    #[derive(Clone, Default)]
    struct SharedStore(Rc<RefCell<Vec<Vec<u8>>>>);

    impl Storage for SharedStore {
        fn append(&mut self, record: &[u8]) -> io::Result<()> {
            self.0.borrow_mut().push(record.to_vec());
            Ok(())
        }

        fn records(&mut self) -> io::Result<Vec<Vec<u8>>> {
            Ok(self.0.borrow().clone())
        }
    }

    fn authority() -> Ed25519Signer {
        Ed25519Signer::from_seed(&SEED)
    }

    fn open(store: &SharedStore) -> RevocationLedger<SharedStore, Ed25519Signer> {
        RevocationLedger::open(store.clone(), &authority().verifier(), authority()).expect("open")
    }

    fn token() -> ConsentToken {
        ConsentToken::issue(&authority(), PATIENT, &[Purpose::Treatment], 10_000)
    }

    #[test]
    fn revoked_token_is_refused_even_with_a_valid_signature() {
        let store = SharedStore::default();
        let mut ledger = open(&store);
        let t = token();
        let verifier = authority().verifier();

        assert_eq!(
            check_with_revocations(
                &t,
                &verifier,
                ledger.revoked(),
                PATIENT,
                Purpose::Treatment,
                1
            ),
            Ok(())
        );
        ledger.revoke(&t, 2).expect("revoke");
        assert_eq!(
            check_with_revocations(
                &t,
                &verifier,
                ledger.revoked(),
                PATIENT,
                Purpose::Treatment,
                3
            ),
            Err(ConsentError::Revoked)
        );
    }

    #[test]
    fn revocation_survives_a_restart() {
        let store = SharedStore::default();
        {
            let mut ledger = open(&store);
            ledger.revoke(&token(), 1).expect("revoke");
        }
        let reopened = open(&store);
        assert!(reopened.revoked().contains(&token().id()));
    }

    #[test]
    fn revoking_twice_writes_one_record() {
        let store = SharedStore::default();
        let mut ledger = open(&store);
        ledger.revoke(&token(), 1).expect("revoke");
        ledger.revoke(&token(), 2).expect("second revoke");
        assert_eq!(store.0.borrow().len(), 1);
    }

    #[test]
    fn revoking_one_token_leaves_another_for_the_same_patient_valid() {
        let store = SharedStore::default();
        let mut ledger = open(&store);
        let revoked = token();
        let other = ConsentToken::issue(&authority(), PATIENT, &[Purpose::Logging], 10_000);
        ledger.revoke(&revoked, 1).expect("revoke");
        assert!(!ledger.revoked().contains(&other.id()));
    }

    #[test]
    fn malformed_revocation_record_stops_the_open() {
        let store = SharedStore::default();
        let signer = authority();
        let mut journal = Journal::open(store.clone(), &signer.verifier()).expect("journal");
        journal
            .append(&signer, vec![KIND_REVOKE, 1, 2])
            .expect("append");
        let err = RevocationLedger::open(store.clone(), &signer.verifier(), authority())
            .expect_err("malformed");
        assert!(matches!(err, RevocationError::Malformed { record: 0 }));
    }
}
