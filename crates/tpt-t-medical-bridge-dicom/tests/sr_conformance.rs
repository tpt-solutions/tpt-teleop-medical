//! Structural conformance checks for the Basic Text SR export (PS3.3 C.17,
//! PS3.5 §7).
//!
//! These checks read the exported bytes back with the crate's own Explicit VR
//! decoder, and test them against the attributes the standard requires. They
//! are not a substitute for a DICOM conformance validator, which needs a target
//! PACS and is still pending. They catch the mistakes this crate can make
//! itself: missing required attributes, out-of-order tags, and wrong values.

use tpt_t_medical_bridge_dicom::net::dataset::{Element, decode_explicit, sequence_items};
use tpt_t_medical_bridge_dicom::net::dimse::BASIC_TEXT_SR;
use tpt_t_medical_bridge_dicom::sr::{Code, Report, export_basic_text_sr};

const PREAMBLE_AND_MAGIC: usize = 132;

const SOP_CLASS: (u16, u16) = (0x0008, 0x0016);
const SOP_INSTANCE: (u16, u16) = (0x0008, 0x0018);
const MODALITY: (u16, u16) = (0x0008, 0x0060);
const STUDY_UID: (u16, u16) = (0x0020, 0x000D);
const SERIES_UID: (u16, u16) = (0x0020, 0x000E);
const SERIES_NUMBER: (u16, u16) = (0x0020, 0x0011);
const INSTANCE_NUMBER: (u16, u16) = (0x0020, 0x0013);
const CONTENT_DATE: (u16, u16) = (0x0008, 0x0023);
const CONTENT_TIME: (u16, u16) = (0x0008, 0x0033);
const CONTINUITY: (u16, u16) = (0x0040, 0xA050);
const COMPLETION: (u16, u16) = (0x0040, 0xA491);
const VERIFICATION: (u16, u16) = (0x0040, 0xA493);
const CONCEPT_NAME: (u16, u16) = (0x0040, 0xA043);
const CONTENT_SEQUENCE: (u16, u16) = (0x0040, 0xA730);
const RELATIONSHIP: (u16, u16) = (0x0040, 0xA010);
const VALUE_TYPE: (u16, u16) = (0x0040, 0xA040);
const TEXT_VALUE: (u16, u16) = (0x0040, 0xA160);
const CODE_VALUE: (u16, u16) = (0x0008, 0x0100);
const CODE_MEANING: (u16, u16) = (0x0008, 0x0104);

/// Type 1 attributes of the SR modules, with the value the export must carry.
const TYPE1_VALUES: [((u16, u16), &str); 8] = [
    (MODALITY, "SR"),
    (CONTINUITY, "SEPARATE"),
    (COMPLETION, "COMPLETE"),
    (VERIFICATION, "UNVERIFIED"),
    (INSTANCE_NUMBER, "1"),
    (SERIES_NUMBER, "1"),
    (CONTENT_DATE, "20260101"),
    (CONTENT_TIME, "120000"),
];

/// Type 1 attributes whose value varies, so only presence is checked.
const TYPE1_PRESENT: [(u16, u16); 4] = [SOP_INSTANCE, STUDY_UID, SERIES_UID, CONCEPT_NAME];

/// Type 2 attributes: the element must be present, though its value may be empty.
const TYPE2_PRESENT: [(u16, u16); 9] = [
    (0x0010, 0x0010), // Patient's Name
    (0x0010, 0x0020), // Patient ID
    (0x0010, 0x0030), // Patient's Birth Date
    (0x0010, 0x0040), // Patient's Sex
    (0x0008, 0x0020), // Study Date
    (0x0008, 0x0030), // Study Time
    (0x0008, 0x0050), // Accession Number
    (0x0008, 0x0090), // Referring Physician's Name
    (0x0020, 0x0010), // Study ID
];

fn report() -> Report {
    Report {
        patient_id: "PID-77".into(),
        study_uid: "1.2.3.4.5".into(),
        series_uid: "1.2.3.4.5.6".into(),
        sop_instance_uid: "1.2.3.4.5.6.7".into(),
        date: "20260101".into(),
        time: "120000".into(),
        title: Code {
            value: "11528-7".into(),
            scheme: "LN".into(),
            meaning: "Radiology Report".into(),
        },
        findings: vec![
            "Anastomosis intact".into(),
            "No bleeding at the closure".into(),
        ],
    }
}

fn exported() -> Vec<u8> {
    export_basic_text_sr(&report()).expect("exports")
}

/// Decodes the whole Part 10 file: the file meta group, then the dataset.
fn decode(bytes: &[u8]) -> Vec<Element> {
    decode_explicit(&bytes[PREAMBLE_AND_MAGIC..]).expect("decodes as Explicit VR")
}

fn find(elements: &[Element], tag: (u16, u16)) -> Option<&Element> {
    elements.iter().find(|e| e.tag == tag)
}

fn text_of(elements: &[Element], tag: (u16, u16)) -> Option<String> {
    find(elements, tag).map(Element::text)
}

/// Decodes the items of a sequence element.
fn items(elements: &[Element], tag: (u16, u16)) -> Vec<Vec<Element>> {
    let el = find(elements, tag).expect("sequence present");
    sequence_items(&el.value)
        .expect("sequence splits into items")
        .into_iter()
        .map(|item| decode_explicit(item).expect("item decodes"))
        .collect()
}

#[test]
fn the_file_meta_names_the_sop_class_and_instance() {
    let els = decode(&exported());
    assert_eq!(
        text_of(&els, (0x0002, 0x0002)).as_deref(),
        Some(BASIC_TEXT_SR)
    );
    assert_eq!(text_of(&els, SOP_CLASS).as_deref(), Some(BASIC_TEXT_SR));
    assert_eq!(
        text_of(&els, (0x0002, 0x0003)).as_deref(),
        Some("1.2.3.4.5.6.7")
    );
    assert_eq!(
        text_of(&els, SOP_INSTANCE).as_deref(),
        Some("1.2.3.4.5.6.7")
    );
}

#[test]
fn every_type_1_attribute_is_present_with_its_required_value() {
    let els = decode(&exported());
    for (tag, want) in TYPE1_VALUES {
        assert_eq!(text_of(&els, tag).as_deref(), Some(want), "tag {tag:04X?}");
    }
}

#[test]
fn type_1_uids_and_the_concept_name_are_present() {
    let els = decode(&exported());
    for tag in TYPE1_PRESENT {
        assert!(find(&els, tag).is_some(), "missing {tag:04X?}");
    }
}

#[test]
fn every_type_2_attribute_is_present_even_when_empty() {
    let els = decode(&exported());
    for tag in TYPE2_PRESENT {
        assert!(find(&els, tag).is_some(), "missing {tag:04X?}");
    }
}

#[test]
fn top_level_tags_are_in_ascending_order() {
    // PS3.5 §7.1: data elements are encoded in ascending tag order. The meta
    // group is first, and the dataset follows it, so the whole file must be
    // strictly ascending from the start of the dataset.
    let els = decode(&exported());
    let meta_end = els
        .iter()
        .position(|e| e.tag.0 != 0x0002)
        .expect("dataset present");
    let dataset = &els[meta_end..];
    for pair in dataset.windows(2) {
        assert!(
            pair[0].tag < pair[1].tag,
            "{:04X?} is not before {:04X?}",
            pair[0].tag,
            pair[1].tag
        );
    }
}

#[test]
fn the_concept_name_is_the_report_title() {
    let els = decode(&exported());
    let title = items(&els, CONCEPT_NAME);
    assert_eq!(title.len(), 1, "one title item");
    assert_eq!(text_of(&title[0], CODE_VALUE).as_deref(), Some("11528-7"));
    assert_eq!(
        text_of(&title[0], CODE_MEANING).as_deref(),
        Some("Radiology Report")
    );
}

#[test]
fn every_finding_is_a_text_item_in_order() {
    let els = decode(&exported());
    let content = items(&els, CONTENT_SEQUENCE);
    let texts: Vec<String> = content
        .iter()
        .map(|item| {
            assert_eq!(
                text_of(item, RELATIONSHIP).as_deref(),
                Some("CONTAINS"),
                "relationship"
            );
            assert_eq!(text_of(item, VALUE_TYPE).as_deref(), Some("TEXT"));
            text_of(item, TEXT_VALUE).expect("text value")
        })
        .collect();
    assert_eq!(
        texts,
        vec!["Anastomosis intact", "No bleeding at the closure"]
    );
}

#[test]
fn a_report_with_no_findings_is_refused_not_exported_empty() {
    let mut r = report();
    r.findings.clear();
    assert!(export_basic_text_sr(&r).is_err());
}
