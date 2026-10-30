//! Cross-bridge conformance checks. See the crate docs for the rules.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::io;
use std::rc::Rc;

use tpt_t_medical_bridge_dicom::{DicomHeader, TransferSyntax};
use tpt_t_medical_bridge_hl7fhir::{AdtEvent, AdtKind};
use tpt_t_medical_compliance::Storage;
use tpt_t_medical_compliance::ed25519::Ed25519Signer;
use tpt_t_medical_compliance::journal::Journal;

const SEED: [u8; 32] = [77; 32];

// ---------------------------------------------------------------- fixtures

fn hl7(trigger: &str, patient: &str, location: Option<&str>) -> Vec<u8> {
    let mut segments = vec![
        format!("MSH|^~\\&|HIS|HOSP|LIS|HOSP|202601011200||ADT^{trigger}|MSG001|P|2.5"),
        format!("PID|1||{patient}^^^HOSP^MR||DOE^JANE|||F"),
    ];
    if let Some(loc) = location {
        segments.push(format!("PV1|1|I|{loc}"));
    }
    segments.join("\r").into_bytes()
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn explicit_element(group: u16, element: u16, vr: &[u8; 2], value: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&group.to_le_bytes());
    out.extend_from_slice(&element.to_le_bytes());
    out.extend_from_slice(vr);
    if matches!(vr, b"OB" | b"OW" | b"UN" | b"UT") {
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(&u32::try_from(value.len()).expect("small").to_le_bytes());
    } else {
        out.extend_from_slice(&u16::try_from(value.len()).expect("small").to_le_bytes());
    }
    out.extend_from_slice(value);
    out
}

/// A padded value, as DICOM requires for even lengths.
fn padded(s: &str) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    if v.len() % 2 == 1 {
        v.push(b' ');
    }
    v
}

/// A small uncompressed CT file for one patient.
fn ct_file(patient: &str) -> Vec<u8> {
    let mut file = vec![0u8; 128];
    file.extend_from_slice(b"DICM");
    file.extend(explicit_element(
        0x0002,
        0x0010,
        b"UI",
        &padded("1.2.840.10008.1.2.1"),
    ));
    file.extend(explicit_element(0x0008, 0x0060, b"CS", &padded("CT")));
    file.extend(explicit_element(0x0010, 0x0020, b"LO", &padded(patient)));
    file.extend(explicit_element(
        0x0020,
        0x000D,
        b"UI",
        &padded("1.2.826.0.1.3680043.2.1"),
    ));
    file.extend(explicit_element(0x0028, 0x0010, b"US", &[0x00, 0x02]));
    file.extend(explicit_element(0x0028, 0x0011, b"US", &[0x00, 0x02]));
    file
}

// ------------------------------------------------------ reference admission

/// The census the rules are checked against. It records who is admitted.
#[derive(Default)]
struct Census {
    admitted: BTreeSet<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum CensusError {
    NotAdmitted,
    AlreadyAdmitted,
}

impl Census {
    fn apply(&mut self, event: &AdtEvent) -> Result<(), CensusError> {
        match event.kind {
            AdtKind::Admit => {
                if !self.admitted.insert(event.patient_id.clone()) {
                    return Err(CensusError::AlreadyAdmitted);
                }
            }
            AdtKind::Transfer => {
                if !self.admitted.contains(&event.patient_id) {
                    return Err(CensusError::NotAdmitted);
                }
            }
            AdtKind::Discharge => {
                if !self.admitted.remove(&event.patient_id) {
                    return Err(CensusError::NotAdmitted);
                }
            }
        }
        Ok(())
    }

    fn is_admitted(&self, patient: &str) -> bool {
        self.admitted.contains(patient)
    }
}

// ------------------------------------------------------------ journal store

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

fn signer() -> Ed25519Signer {
    Ed25519Signer::from_seed(&SEED)
}

/// One journal payload per ADT event: a kind byte, then the identifier.
fn adt_payload(event: &AdtEvent) -> Vec<u8> {
    let kind = match event.kind {
        AdtKind::Admit => 1u8,
        AdtKind::Transfer => 2,
        AdtKind::Discharge => 3,
    };
    let mut out = vec![kind];
    out.extend_from_slice(event.patient_id.as_bytes());
    out
}

// -------------------------------------------------------------------- tests

#[test]
fn admit_transfer_discharge_is_accepted_and_journalled() {
    let store = SharedStore::default();
    let s = signer();
    let mut journal = Journal::open(store.clone(), &s.verifier()).expect("open");
    let mut census = Census::default();

    let messages = [
        hl7("A01", "MRN-1", Some("WARD4^401")),
        hl7("A02", "MRN-1", Some("ICU^1")),
        hl7("A03", "MRN-1", Some("ICU^1")),
    ];
    for bytes in &messages {
        let event = tpt_t_medical_bridge_hl7fhir::parse(bytes).expect("valid ADT");
        census.apply(&event).expect("rule holds");
        journal.append(&s, adt_payload(&event)).expect("journal");
    }

    assert!(!census.is_admitted("MRN-1"), "discharged at the end");
    let reopened = Journal::open(store, &s.verifier()).expect("reopen verifies");
    assert_eq!(reopened.log().entries().len(), 3);
    assert_eq!(
        reopened.log().entries()[1].payload[0],
        2,
        "second record is the transfer"
    );
}

#[test]
fn transfer_before_admit_breaks_the_rule() {
    let mut census = Census::default();
    let event = tpt_t_medical_bridge_hl7fhir::parse(&hl7("A02", "MRN-9", Some("ICU^1")))
        .expect("valid ADT");
    assert_eq!(census.apply(&event), Err(CensusError::NotAdmitted));
}

#[test]
fn discharge_without_admit_breaks_the_rule() {
    let mut census = Census::default();
    let event = tpt_t_medical_bridge_hl7fhir::parse(&hl7("A03", "MRN-9", None)).expect("valid ADT");
    assert_eq!(census.apply(&event), Err(CensusError::NotAdmitted));
}

#[test]
fn double_admit_breaks_the_rule() {
    let mut census = Census::default();
    let admit = tpt_t_medical_bridge_hl7fhir::parse(&hl7("A01", "MRN-2", None)).expect("valid");
    census.apply(&admit).expect("first admit");
    assert_eq!(census.apply(&admit), Err(CensusError::AlreadyAdmitted));
}

#[test]
fn ct_scan_matches_only_the_admitted_patient() {
    let mut census = Census::default();
    let admit =
        tpt_t_medical_bridge_hl7fhir::parse(&hl7("A01", "MRN-5", Some("WARD4^1"))).expect("valid");
    census.apply(&admit).expect("admitted");

    let scan: DicomHeader = tpt_t_medical_bridge_dicom::parse(&ct_file("MRN-5")).expect("valid CT");
    assert_eq!(scan.transfer_syntax, TransferSyntax::ExplicitLittleEndian);
    assert!(
        census.is_admitted(&scan.patient_id),
        "scan matches an admitted patient"
    );

    let stranger: DicomHeader =
        tpt_t_medical_bridge_dicom::parse(&ct_file("MRN-6")).expect("valid CT");
    assert!(
        !census.is_admitted(&stranger.patient_id),
        "a scan of an unadmitted patient must not be matched"
    );
}

#[test]
fn hostile_input_from_either_bridge_never_panics() {
    let base_hl7 = hl7("A01", "MRN-1", Some("WARD4^401"));
    let base_dicom = ct_file("MRN-1");

    for i in 0..base_hl7.len() {
        let _ = tpt_t_medical_bridge_hl7fhir::parse(&base_hl7[..i]);
    }
    for i in 0..base_dicom.len() {
        let _ = tpt_t_medical_bridge_dicom::parse(&base_dicom[..i]);
    }

    // A simple, repeatable pseudo-random walk over byte changes.
    let mut state = 0x1234_5678_9abc_def0u64;
    let mut next = move || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        state >> 33
    };
    for _ in 0..5_000 {
        let mut a = base_hl7.clone();
        let mut b = base_dicom.clone();
        for _ in 0..3 {
            let ia = usize::try_from(next()).unwrap_or(0) % a.len();
            a[ia] = u8::try_from(next() % 256).unwrap_or(0);
            let ib = usize::try_from(next()).unwrap_or(0) % b.len();
            b[ib] = u8::try_from(next() % 256).unwrap_or(0);
        }
        let _ = tpt_t_medical_bridge_hl7fhir::parse(&a);
        let _ = tpt_t_medical_bridge_dicom::parse(&b);
    }
}

#[test]
fn debug_output_from_both_bridges_hides_identifiers() {
    let event = tpt_t_medical_bridge_hl7fhir::parse(&hl7("A01", "MRN-SECRET", Some("WARD-SECRET")))
        .expect("valid");
    let scan = tpt_t_medical_bridge_dicom::parse(&ct_file("MRN-SECRET")).expect("valid CT");

    for shown in [format!("{event:?}"), format!("{scan:?}")] {
        assert!(!shown.contains("MRN-SECRET"), "{shown}");
        assert!(!shown.contains("WARD-SECRET"), "{shown}");
    }
}

#[test]
fn journal_detects_a_tampered_admission_record_after_restart() {
    let store = SharedStore::default();
    let s = signer();
    {
        let mut journal = Journal::open(store.clone(), &s.verifier()).expect("open");
        let event = tpt_t_medical_bridge_hl7fhir::parse(&hl7("A01", "MRN-3", None)).expect("valid");
        journal.append(&s, adt_payload(&event)).expect("journal");
    }
    // Flip the last byte of the stored record.
    {
        let mut records = store.0.borrow_mut();
        let last = records[0].len() - 1;
        records[0][last] ^= 0x01;
    }
    assert!(Journal::open(store, &s.verifier()).is_err());
}

#[test]
fn medication_order_is_accepted_only_for_an_admitted_patient() {
    let mut census = Census::default();
    let admit = tpt_t_medical_bridge_hl7fhir::parse(&hl7("A01", "MRN-8", Some("WARD4^1")))
        .expect("valid ADT");
    census.apply(&admit).expect("admitted");

    let order = br#"{"resourceType":"MedicationRequest","id":"rx-1","status":"active",
        "subject":{"reference":"Patient/MRN-8"},
        "medicationCodeableConcept":{"text":"heparin"}}"#;
    let parsed =
        tpt_t_medical_bridge_hl7fhir::fhir::parse_medication_request(order).expect("valid order");
    assert!(
        census.is_admitted(&parsed.patient_id),
        "order for an admitted patient"
    );

    let stranger = br#"{"resourceType":"MedicationRequest","id":"rx-2","status":"active",
        "subject":{"reference":"Patient/MRN-9"},
        "medicationCodeableConcept":{"text":"heparin"}}"#;
    let parsed = tpt_t_medical_bridge_hl7fhir::fhir::parse_medication_request(stranger)
        .expect("valid order");
    assert!(
        !census.is_admitted(&parsed.patient_id),
        "order for an unadmitted patient"
    );
}
