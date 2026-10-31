//! DICOM Structured Report export for post-op reports (spec.txt §4.4, Phase 6).
//!
//! The export is a Basic Text SR (SOP class `1.2.840.10008.5.1.4.1.1.88.11`):
//! a document title, and one TEXT content item per finding, each under the
//! DCM "Comment" concept. The output is a Part 10 file with Explicit VR little
//! endian, so it can be stored with C-STORE (see [`crate::net::Association`])
//! or written to disk.
//!
//! This is a minimal Basic Text SR. It carries the attributes the SR module
//! requires, and it has not been run through a DICOM conformance validator.
//! That needs the target PACS, and it is listed as pending in the checklist.
//!
//! Inputs are validated before anything is written, because they come from the
//! clinical record: UIDs must be well-formed, and text is bounded.

use core::fmt;

use crate::net::dataset::{encode_explicit, sequence_element, sequence_item};
use crate::net::dimse::BASIC_TEXT_SR;
use crate::net::pdu::TS_EXPLICIT_LE;

/// Most findings in one report.
pub const MAX_FINDINGS: usize = 256;
/// Longest finding, in bytes.
pub const MAX_FINDING_BYTES: usize = 4_096;
/// Longest patient identifier, in bytes.
pub const MAX_PATIENT_BYTES: usize = 64;

const PREAMBLE_LEN: usize = 128;

/// A coded concept, as (value, scheme, meaning).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Code {
    /// The code value.
    pub value: String,
    /// The coding scheme designator, such as `DCM` or `LN`.
    pub scheme: String,
    /// The human-readable meaning.
    pub meaning: String,
}

/// A post-op report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The patient identifier.
    pub patient_id: String,
    /// The study the report belongs to.
    pub study_uid: String,
    /// The series the report is filed in.
    pub series_uid: String,
    /// This report's own instance UID.
    pub sop_instance_uid: String,
    /// The report date, as `YYYYMMDD`.
    pub date: String,
    /// The report time, as `HHMMSS`.
    pub time: String,
    /// The document title.
    pub title: Code,
    /// The findings, in order.
    pub findings: Vec<String>,
}

/// Why a report cannot be exported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SrError {
    /// A UID is not well formed.
    BadUid(&'static str),
    /// The patient identifier is empty or too long.
    BadPatient,
    /// A date or time is not in the DICOM form.
    BadDateTime,
    /// There are no findings, or too many.
    BadFindingCount(usize),
    /// A finding is empty or too long.
    BadFinding(usize),
}

impl fmt::Display for SrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadUid(what) => write!(f, "{what} is not a valid UID"),
            Self::BadPatient => f.write_str("patient identifier is empty or too long"),
            Self::BadDateTime => f.write_str("date or time is not in DICOM form"),
            Self::BadFindingCount(n) => {
                write!(f, "{n} findings, which is outside 1 to {MAX_FINDINGS}")
            }
            Self::BadFinding(i) => write!(f, "finding {i} is empty or too long"),
        }
    }
}

impl std::error::Error for SrError {}

/// Checks a UID: digits and dots, at most 64 characters, no empty component, and
/// no leading zero in a component longer than one digit (PS3.5 §9.1).
fn valid_uid(uid: &str) -> bool {
    if uid.is_empty() || uid.len() > 64 {
        return false;
    }
    uid.split('.').all(|part| {
        !part.is_empty()
            && part.bytes().all(|b| b.is_ascii_digit())
            && !(part.len() > 1 && part.starts_with('0'))
    })
}

fn check_uid(uid: &str, what: &'static str) -> Result<(), SrError> {
    if valid_uid(uid) {
        Ok(())
    } else {
        Err(SrError::BadUid(what))
    }
}

fn valid_date(s: &str) -> bool {
    s.len() == 8 && s.bytes().all(|b| b.is_ascii_digit())
}

fn valid_time(s: &str) -> bool {
    s.len() == 6 && s.bytes().all(|b| b.is_ascii_digit())
}

/// Checks a report and returns its Part 10 bytes.
///
/// # Errors
///
/// Returns an error naming the first field that fails its check.
pub fn export_basic_text_sr(report: &Report) -> Result<Vec<u8>, SrError> {
    check_uid(&report.study_uid, "study UID")?;
    check_uid(&report.series_uid, "series UID")?;
    check_uid(&report.sop_instance_uid, "SOP instance UID")?;
    if report.patient_id.is_empty() || report.patient_id.len() > MAX_PATIENT_BYTES {
        return Err(SrError::BadPatient);
    }
    if !valid_date(&report.date) || !valid_time(&report.time) {
        return Err(SrError::BadDateTime);
    }
    if report.findings.is_empty() || report.findings.len() > MAX_FINDINGS {
        return Err(SrError::BadFindingCount(report.findings.len()));
    }
    for (i, f) in report.findings.iter().enumerate() {
        if f.is_empty() || f.len() > MAX_FINDING_BYTES {
            return Err(SrError::BadFinding(i));
        }
    }

    let items: Vec<Vec<u8>> = report
        .findings
        .iter()
        .map(|text| sequence_item(&text_item(text)))
        .collect();
    let mut dataset = encode_explicit(&[
        ((0x0008, 0x0016), *b"UI", BASIC_TEXT_SR.as_bytes().to_vec()),
        (
            (0x0008, 0x0018),
            *b"UI",
            report.sop_instance_uid.as_bytes().to_vec(),
        ),
        ((0x0008, 0x0023), *b"DA", report.date.as_bytes().to_vec()),
        ((0x0008, 0x0033), *b"TM", report.time.as_bytes().to_vec()),
        ((0x0008, 0x0060), *b"CS", b"SR".to_vec()),
        (
            (0x0010, 0x0020),
            *b"LO",
            report.patient_id.as_bytes().to_vec(),
        ),
        (
            (0x0020, 0x000D),
            *b"UI",
            report.study_uid.as_bytes().to_vec(),
        ),
        (
            (0x0020, 0x000E),
            *b"UI",
            report.series_uid.as_bytes().to_vec(),
        ),
        ((0x0020, 0x0013), *b"IS", b"1".to_vec()),
        ((0x0040, 0xA050), *b"CS", b"SEPARATE".to_vec()),
        ((0x0040, 0xA491), *b"CS", b"COMPLETE".to_vec()),
        ((0x0040, 0xA493), *b"CS", b"UNVERIFIED".to_vec()),
    ]);
    dataset.extend(concept_name_sequence((0x0040, 0xA043), &report.title));
    dataset.extend(sequence_element((0x0040, 0xA730), &items));

    let meta = meta_group(&report.sop_instance_uid);
    let mut out = vec![0_u8; PREAMBLE_LEN];
    out.extend_from_slice(b"DICM");
    out.extend_from_slice(&meta);
    out.extend_from_slice(&dataset);
    Ok(out)
}

/// The TEXT content item for one finding, without its item wrapper.
fn text_item(text: &str) -> Vec<u8> {
    let comment = Code {
        value: "121106".into(),
        scheme: "DCM".into(),
        meaning: "Comment".into(),
    };
    let mut content = encode_explicit(&[
        ((0x0040, 0xA010), *b"CS", b"CONTAINS".to_vec()),
        ((0x0040, 0xA040), *b"CS", b"TEXT".to_vec()),
    ]);
    content.extend(concept_name_sequence((0x0040, 0xA043), &comment));
    content.extend(encode_explicit(&[(
        (0x0040, 0xA160),
        *b"UT",
        text.as_bytes().to_vec(),
    )]));
    content
}

/// A sequence element holding one code item, such as a concept name.
fn concept_name_sequence(tag: (u16, u16), code: &Code) -> Vec<u8> {
    let content = encode_explicit(&[
        ((0x0008, 0x0100), *b"SH", code.value.as_bytes().to_vec()),
        ((0x0008, 0x0102), *b"SH", code.scheme.as_bytes().to_vec()),
        ((0x0008, 0x0104), *b"LO", code.meaning.as_bytes().to_vec()),
    ]);
    sequence_element(tag, &[sequence_item(&content)])
}

/// The File Meta Information group, with its own group length.
fn meta_group(sop_instance_uid: &str) -> Vec<u8> {
    let body = encode_explicit(&[
        ((0x0002, 0x0001), *b"OB", vec![0, 1]),
        ((0x0002, 0x0002), *b"UI", BASIC_TEXT_SR.as_bytes().to_vec()),
        (
            (0x0002, 0x0003),
            *b"UI",
            sop_instance_uid.as_bytes().to_vec(),
        ),
        ((0x0002, 0x0010), *b"UI", TS_EXPLICIT_LE.as_bytes().to_vec()),
        (
            (0x0002, 0x0012),
            *b"UI",
            crate::net::pdu::IMPLEMENTATION_UID.as_bytes().to_vec(),
        ),
    ]);
    let length = u32::try_from(body.len()).unwrap_or(u32::MAX);
    let mut out = encode_explicit(&[((0x0002, 0x0000), *b"UL", length.to_le_bytes().to_vec())]);
    out.extend_from_slice(&body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::dataset::{Element, decode_explicit, sequence_items};

    fn report() -> Report {
        Report {
            patient_id: "MRN-42".into(),
            study_uid: "1.2.826.0.1.3680043.2.1143.100".into(),
            series_uid: "1.2.826.0.1.3680043.2.1143.101".into(),
            sop_instance_uid: "1.2.826.0.1.3680043.2.1143.102".into(),
            date: "20260612".into(),
            time: "093015".into(),
            title: Code {
                value: "18748-4".into(),
                scheme: "LN".into(),
                meaning: "Diagnostic imaging report".into(),
            },
            findings: vec![
                "Port sites clean and dry.".into(),
                "No haemorrhage on the post-op scan.".into(),
            ],
        }
    }

    fn element(elements: &[Element], tag: (u16, u16)) -> Option<&Element> {
        elements.iter().find(|e| e.tag == tag)
    }

    #[test]
    fn the_file_has_a_preamble_and_a_meta_group_naming_the_sop_class() {
        let bytes = export_basic_text_sr(&report()).expect("export");
        assert_eq!(&bytes[128..132], b"DICM");
        let meta_start = 132;
        let meta = decode_explicit(&bytes[meta_start..]).expect("meta");
        assert_eq!(meta[0].tag, (0x0002, 0x0000));
        let class = element(&meta, (0x0002, 0x0002)).expect("class");
        assert_eq!(class.text(), BASIC_TEXT_SR);
    }

    #[test]
    fn the_dataset_carries_the_patient_study_and_each_finding() {
        let bytes = export_basic_text_sr(&report()).expect("export");
        let meta_len = {
            let meta = decode_explicit(&bytes[132..]).expect("meta");
            // The group length element is the first, 12 bytes, then the group.
            let len = u32::from_le_bytes([
                meta[0].value[0],
                meta[0].value[1],
                meta[0].value[2],
                meta[0].value[3],
            ]);
            12 + len as usize
        };
        let dataset = decode_explicit(&bytes[132 + meta_len..]).expect("dataset");
        assert_eq!(
            element(&dataset, (0x0010, 0x0020)).expect("patient").text(),
            "MRN-42"
        );
        assert_eq!(
            element(&dataset, (0x0008, 0x0060))
                .expect("modality")
                .text(),
            "SR"
        );
        let seq = element(&dataset, (0x0040, 0xA730)).expect("content");
        let items = sequence_items(&seq.value).expect("items");
        assert_eq!(items.len(), 2);
        let first = decode_explicit(items[0]).expect("item");
        assert_eq!(
            element(&first, (0x0040, 0xA160)).expect("text").text(),
            "Port sites clean and dry."
        );
    }

    #[test]
    fn malformed_uids_are_refused_before_export() {
        let mut r = report();
        r.study_uid = "1.2.03".into();
        assert_eq!(export_basic_text_sr(&r), Err(SrError::BadUid("study UID")));
        r.study_uid = "1..2".into();
        assert_eq!(export_basic_text_sr(&r), Err(SrError::BadUid("study UID")));
        r.study_uid = "1.2.x".into();
        assert_eq!(export_basic_text_sr(&r), Err(SrError::BadUid("study UID")));
    }

    #[test]
    fn empty_and_oversized_fields_are_refused() {
        let mut r = report();
        r.findings.clear();
        assert_eq!(export_basic_text_sr(&r), Err(SrError::BadFindingCount(0)));
        let mut r = report();
        r.findings[1] = "x".repeat(MAX_FINDING_BYTES + 1);
        assert_eq!(export_basic_text_sr(&r), Err(SrError::BadFinding(1)));
        let mut r = report();
        r.patient_id.clear();
        assert_eq!(export_basic_text_sr(&r), Err(SrError::BadPatient));
        let mut r = report();
        r.time = "9:30".into();
        assert_eq!(export_basic_text_sr(&r), Err(SrError::BadDateTime));
    }
}
