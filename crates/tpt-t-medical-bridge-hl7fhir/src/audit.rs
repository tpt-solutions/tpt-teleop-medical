//! Audit posting for the bridges (spec.txt §4.4).
//!
//! Each ADT event and each medication order is written to the compliance
//! journal, signed and hash-chained, before the caller acts on it. The record
//! holds the message kind and the opaque identifiers, not clinical content.
//!
//! The patient identifier is kept in the audit record. Auditing needs to say
//! whose data was touched. Names and other PHI are never read by the bridge, so
//! they never reach the journal.

use std::fmt;

use tpt_t_medical_compliance::journal::{Journal, JournalError};
use tpt_t_medical_compliance::{Signer, Storage, Verifier};

use crate::AdtEvent;
use crate::fhir::MedicationOrder;

const KIND_ADT: u8 = b'H';
const KIND_ORDER: u8 = b'F';

/// Journals bridge events.
pub struct BridgeAudit<S: Storage, G: Signer> {
    journal: Journal<S>,
    signer: G,
}

impl<S: Storage, G: Signer> fmt::Debug for BridgeAudit<S, G> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BridgeAudit")
            .field("records", &self.journal.log().entries().len())
            .finish_non_exhaustive()
    }
}

impl<S: Storage, G: Signer> BridgeAudit<S, G> {
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

    /// Returns the journal of bridge records.
    #[must_use]
    pub const fn journal(&self) -> &Journal<S> {
        &self.journal
    }

    /// Records an ADT event and returns its sequence number.
    ///
    /// # Errors
    ///
    /// Returns an error if the record cannot be written.
    pub fn record_adt(&mut self, event: &AdtEvent) -> Result<u64, JournalError> {
        let kind = match event.kind {
            crate::AdtKind::Admit => "admit",
            crate::AdtKind::Transfer => "transfer",
            crate::AdtKind::Discharge => "discharge",
        };
        let mut payload = vec![KIND_ADT];
        payload.extend_from_slice(format!("{kind}|{}", event.patient_id).as_bytes());
        self.journal.append(&self.signer, payload)
    }

    /// Records a medication order and returns its sequence number.
    ///
    /// # Errors
    ///
    /// Returns an error if the record cannot be written.
    pub fn record_order(&mut self, order: &MedicationOrder) -> Result<u64, JournalError> {
        let mut payload = vec![KIND_ORDER];
        payload.extend_from_slice(
            format!("{}|{}|{}", order.id, order.patient_id, order.status).as_bytes(),
        );
        self.journal.append(&self.signer, payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fhir::parse_medication_request;
    use crate::parse;
    use std::cell::RefCell;
    use std::io;
    use std::rc::Rc;
    use tpt_t_medical_compliance::ed25519::Ed25519Signer;

    const SEED: [u8; 32] = [88; 32];

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

    fn open(store: &SharedStore) -> BridgeAudit<SharedStore, Ed25519Signer> {
        let signer = Ed25519Signer::from_seed(&SEED);
        BridgeAudit::open(store.clone(), &signer.verifier(), signer).expect("open")
    }

    fn admit_event() -> AdtEvent {
        parse(b"MSH|^~\\&|A|B|C|D|t||ADT^A01|1|P|2.5\rPID|1||MRN-7").expect("valid")
    }

    fn order() -> MedicationOrder {
        parse_medication_request(
            br#"{"resourceType":"MedicationRequest","id":"rx-9","status":"active",
                "subject":{"reference":"Patient/MRN-7"},
                "medicationCodeableConcept":{"text":"heparin"}}"#,
        )
        .expect("valid")
    }

    #[test]
    fn adt_and_order_records_are_journalled_and_verify_after_reopen() {
        let store = SharedStore::default();
        {
            let mut audit = open(&store);
            assert_eq!(audit.record_adt(&admit_event()).expect("adt"), 0);
            assert_eq!(audit.record_order(&order()).expect("order"), 1);
        }
        let signer = Ed25519Signer::from_seed(&SEED);
        let reopened = Journal::open(store, &signer.verifier()).expect("chain verifies");
        let entries = reopened.log().entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].payload, b"Hadmit|MRN-7");
        assert_eq!(entries[1].payload, b"Frx-9|MRN-7|active");
    }

    #[test]
    fn debug_output_does_not_show_the_journal_contents() {
        let store = SharedStore::default();
        let mut audit = open(&store);
        audit.record_adt(&admit_event()).expect("adt");
        let shown = format!("{audit:?}");
        assert!(!shown.contains("MRN-7"), "{shown}");
        assert!(shown.contains("records"));
    }
}
