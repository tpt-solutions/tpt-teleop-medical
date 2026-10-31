//! DICOM metadata ingestion for tpt-teleop-medical (Phase 6).
//!
//! This crate reads the metadata of an uncompressed DICOM Part 10 file. It
//! extracts the patient identifier, study UID, modality, and image size. These
//! are what the vision pipeline needs to match a scan to a patient.
//!
//! Scope, stated plainly:
//!
//! * Supported transfer syntaxes: Explicit VR Little Endian and Implicit VR
//!   Little Endian. Anything else, including compressed syntaxes, is refused.
//! * Network services are in [`net`]: the calling side of C-ECHO, C-FIND, C-GET
//!   and C-STORE. C-MOVE is not implemented, because it needs an inbound
//!   listener. The listening side is not implemented. Retrieval uses C-GET.
//! * [`sr`] exports a Basic Text Structured Report for a post-op report.
//! * Sequences are skipped by length and not entered. Undefined-length elements
//!   are refused, because they cannot be skipped safely without parsing their
//!   contents.
//!
//! Pixel data is never read. It is skipped by length.
//!
//! Input comes from outside the hospital, so every size is bounded, and
//! malformed input returns an error rather than panicking.

#![forbid(unsafe_code)]

use core::fmt;

pub mod net;
pub mod sr;

/// Largest file accepted, in bytes.
pub const MAX_FILE_BYTES: usize = 512 * 1024 * 1024;
/// Most data elements visited in one file.
pub const MAX_ELEMENTS: usize = 100_000;

const PREAMBLE_LEN: usize = 128;
const MAGIC: &[u8; 4] = b"DICM";
const TS_EXPLICIT_LE: &str = "1.2.840.10008.1.2.1";
const TS_IMPLICIT_LE: &str = "1.2.840.10008.1.2";

const TAG_TRANSFER_SYNTAX: (u16, u16) = (0x0002, 0x0010);
const TAG_MODALITY: (u16, u16) = (0x0008, 0x0060);
const TAG_PATIENT_ID: (u16, u16) = (0x0010, 0x0020);
const TAG_STUDY_UID: (u16, u16) = (0x0020, 0x000D);
const TAG_ROWS: (u16, u16) = (0x0028, 0x0010);
const TAG_COLUMNS: (u16, u16) = (0x0028, 0x0011);

/// The transfer syntaxes this crate can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferSyntax {
    /// Explicit VR Little Endian.
    ExplicitLittleEndian,
    /// Implicit VR Little Endian.
    ImplicitLittleEndian,
}

/// The metadata this crate extracts.
#[derive(Clone, PartialEq, Eq)]
pub struct DicomHeader {
    /// How the data elements are encoded.
    pub transfer_syntax: TransferSyntax,
    /// Patient identifier. Treat it as PHI.
    pub patient_id: String,
    /// Study instance UID.
    pub study_uid: String,
    /// Modality, such as `CT` or `MR`.
    pub modality: String,
    /// Image rows.
    pub rows: u16,
    /// Image columns.
    pub columns: u16,
}

/// `Debug` redacts the identifiers.
impl fmt::Debug for DicomHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DicomHeader")
            .field("transfer_syntax", &self.transfer_syntax)
            .field("patient_id", &"<redacted>")
            .field("study_uid", &"<redacted>")
            .field("modality", &self.modality)
            .field("rows", &self.rows)
            .field("columns", &self.columns)
            .finish()
    }
}

/// Why a file was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// The file is longer than [`MAX_FILE_BYTES`].
    TooLarge {
        /// The file length, in bytes.
        len: usize,
    },
    /// The file has no 128-byte preamble followed by `DICM`.
    NoPreamble,
    /// The data ends in the middle of an element.
    Truncated {
        /// Byte offset where the data ran out.
        offset: usize,
    },
    /// An element has an undefined length, which this crate does not handle.
    UndefinedLength {
        /// Byte offset of the element.
        offset: usize,
    },
    /// The transfer syntax is not one this crate reads.
    UnsupportedTransferSyntax(String),
    /// The file has more than [`MAX_ELEMENTS`] data elements.
    TooManyElements,
    /// A required element is missing.
    MissingElement(&'static str),
    /// A required element has a value that cannot be read.
    BadValue(&'static str),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge { len } => write!(f, "file is {len} bytes, above the limit"),
            Self::NoPreamble => f.write_str("file has no DICOM preamble"),
            Self::Truncated { offset } => write!(f, "data ends at byte {offset}"),
            Self::UndefinedLength { offset } => {
                write!(f, "element at byte {offset} has an undefined length")
            }
            Self::UnsupportedTransferSyntax(uid) => {
                write!(f, "transfer syntax {uid} is not supported")
            }
            Self::TooManyElements => f.write_str("file has too many data elements"),
            Self::MissingElement(name) => write!(f, "required element {name} is missing"),
            Self::BadValue(name) => write!(f, "element {name} has an unreadable value"),
        }
    }
}

impl std::error::Error for ParseError {}

/// One element's header and where its value sits in the file.
#[derive(Debug, Clone, Copy)]
struct Element {
    tag: (u16, u16),
    value_start: usize,
    value_len: usize,
    next: usize,
}

/// Parses the metadata of a DICOM Part 10 file.
///
/// # Errors
///
/// Returns the first problem found.
pub fn parse(file: &[u8]) -> Result<DicomHeader, ParseError> {
    if file.len() > MAX_FILE_BYTES {
        return Err(ParseError::TooLarge { len: file.len() });
    }
    if file.len() < PREAMBLE_LEN + MAGIC.len()
        || file.get(PREAMBLE_LEN..PREAMBLE_LEN + 4) != Some(MAGIC.as_slice())
    {
        return Err(ParseError::NoPreamble);
    }

    // File meta information is always Explicit VR Little Endian. Its only job
    // here is to name the transfer syntax of the dataset that follows.
    let mut pos = PREAMBLE_LEN + MAGIC.len();
    let mut uid = None;
    while let Ok(el) = read_explicit(file, pos) {
        if el.tag.0 != 0x0002 {
            break;
        }
        if el.tag == TAG_TRANSFER_SYNTAX {
            uid = Some(read_text(file, &el, "TransferSyntaxUID")?);
        }
        pos = el.next;
    }
    let uid = uid.ok_or(ParseError::MissingElement("TransferSyntaxUID"))?;
    let syntax = match uid.as_str() {
        TS_EXPLICIT_LE => TransferSyntax::ExplicitLittleEndian,
        TS_IMPLICIT_LE => TransferSyntax::ImplicitLittleEndian,
        _ => return Err(ParseError::UnsupportedTransferSyntax(uid)),
    };

    let mut found = Found::default();
    let mut count = 0usize;
    while pos < file.len() {
        count += 1;
        if count > MAX_ELEMENTS {
            return Err(ParseError::TooManyElements);
        }
        let el = match syntax {
            TransferSyntax::ExplicitLittleEndian => read_explicit(file, pos)?,
            TransferSyntax::ImplicitLittleEndian => read_implicit(file, pos)?,
        };
        found.take(file, &el)?;
        pos = el.next;
    }

    found.finish(syntax)
}

#[derive(Default)]
struct Found {
    patient_id: Option<String>,
    study_uid: Option<String>,
    modality: Option<String>,
    rows: Option<u16>,
    columns: Option<u16>,
}

impl Found {
    /// Keeps the value of the elements this crate needs. Skips the rest.
    fn take(&mut self, file: &[u8], el: &Element) -> Result<(), ParseError> {
        match el.tag {
            TAG_PATIENT_ID => self.patient_id = Some(read_text(file, el, "PatientID")?),
            TAG_STUDY_UID => self.study_uid = Some(read_text(file, el, "StudyInstanceUID")?),
            TAG_MODALITY => self.modality = Some(read_text(file, el, "Modality")?),
            TAG_ROWS => self.rows = Some(read_u16(file, el, "Rows")?),
            TAG_COLUMNS => self.columns = Some(read_u16(file, el, "Columns")?),
            _ => {}
        }
        Ok(())
    }

    fn finish(self, syntax: TransferSyntax) -> Result<DicomHeader, ParseError> {
        Ok(DicomHeader {
            transfer_syntax: syntax,
            patient_id: self
                .patient_id
                .ok_or(ParseError::MissingElement("PatientID"))?,
            study_uid: self
                .study_uid
                .ok_or(ParseError::MissingElement("StudyInstanceUID"))?,
            modality: self
                .modality
                .ok_or(ParseError::MissingElement("Modality"))?,
            rows: self.rows.ok_or(ParseError::MissingElement("Rows"))?,
            columns: self.columns.ok_or(ParseError::MissingElement("Columns"))?,
        })
    }
}

/// Reads an Explicit VR element header at `pos`.
fn read_explicit(file: &[u8], pos: usize) -> Result<Element, ParseError> {
    let group = read_u16_at(file, pos)?;
    let element = read_u16_at(file, pos + 2)?;
    let vr = file
        .get(pos + 4..pos + 6)
        .ok_or(ParseError::Truncated { offset: pos })?;
    #[allow(clippy::unnested_or_patterns)]
    let long_form = matches!(
        vr,
        b"OB"
            | b"OD"
            | b"OF"
            | b"OL"
            | b"OV"
            | b"OW"
            | b"SQ"
            | b"UC"
            | b"UN"
            | b"UR"
            | b"UT"
            | b"SV"
            | b"UV"
    );
    let (len, header) = if long_form {
        // Two reserved bytes, then a 32-bit length.
        (u32_at(file, pos + 8)?, 12)
    } else {
        (u32::from(read_u16_at(file, pos + 6)?), 8)
    };
    finish_header(file, pos, (group, element), len, header)
}

/// Reads an Implicit VR element header at `pos`.
fn read_implicit(file: &[u8], pos: usize) -> Result<Element, ParseError> {
    let group = read_u16_at(file, pos)?;
    let element = read_u16_at(file, pos + 2)?;
    let len = u32_at(file, pos + 4)?;
    finish_header(file, pos, (group, element), len, 8)
}

fn finish_header(
    file: &[u8],
    pos: usize,
    tag: (u16, u16),
    len: u32,
    header: usize,
) -> Result<Element, ParseError> {
    if len == u32::MAX {
        return Err(ParseError::UndefinedLength { offset: pos });
    }
    let value_start = pos
        .checked_add(header)
        .ok_or(ParseError::Truncated { offset: pos })?;
    let value_len = usize::try_from(len).map_err(|_| ParseError::Truncated { offset: pos })?;
    let next = value_start
        .checked_add(value_len)
        .filter(|&end| end <= file.len())
        .ok_or(ParseError::Truncated { offset: pos })?;
    Ok(Element {
        tag,
        value_start,
        value_len,
        next,
    })
}

fn read_text(file: &[u8], el: &Element, name: &'static str) -> Result<String, ParseError> {
    let bytes = &file[el.value_start..el.value_start + el.value_len];
    let text = std::str::from_utf8(bytes).map_err(|_| ParseError::BadValue(name))?;
    // DICOM pads values with spaces or NUL to an even length.
    Ok(text.trim_end_matches([' ', '\0']).to_string())
}

fn read_u16(file: &[u8], el: &Element, name: &'static str) -> Result<u16, ParseError> {
    if el.value_len != 2 {
        return Err(ParseError::BadValue(name));
    }
    read_u16_at(file, el.value_start).map_err(|_| ParseError::BadValue(name))
}

fn read_u16_at(file: &[u8], pos: usize) -> Result<u16, ParseError> {
    let b = file
        .get(pos..pos + 2)
        .ok_or(ParseError::Truncated { offset: pos })?;
    Ok(u16::from_le_bytes([b[0], b[1]]))
}

fn u32_at(file: &[u8], pos: usize) -> Result<u32, ParseError> {
    let b = file
        .get(pos..pos + 4)
        .ok_or(ParseError::Truncated { offset: pos })?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writes an element with an Explicit VR header. Long-form VRs get the
    /// reserved bytes and a 32-bit length.
    #[allow(clippy::trivially_copy_pass_by_ref)]
    fn explicit(group: u16, element: u16, vr: &[u8; 2], value: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&group.to_le_bytes());
        out.extend_from_slice(&element.to_le_bytes());
        out.extend_from_slice(vr);
        let long = [b"OB", b"OW", b"SQ", b"UN", b"UT"].contains(&vr);
        if long {
            out.extend_from_slice(&[0, 0]);
            out.extend_from_slice(&u32::try_from(value.len()).expect("small").to_le_bytes());
        } else {
            out.extend_from_slice(&u16::try_from(value.len()).expect("small").to_le_bytes());
        }
        out.extend_from_slice(value);
        out
    }

    fn implicit(group: u16, element: u16, value: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&group.to_le_bytes());
        out.extend_from_slice(&element.to_le_bytes());
        out.extend_from_slice(&u32::try_from(value.len()).expect("small").to_le_bytes());
        out.extend_from_slice(value);
        out
    }

    /// Pads a string to an even length, as DICOM requires.
    fn text(s: &str) -> Vec<u8> {
        let mut v = s.as_bytes().to_vec();
        if v.len() % 2 == 1 {
            v.push(b' ');
        }
        v
    }

    fn preamble(transfer_syntax: &str) -> Vec<u8> {
        let mut out = vec![0u8; PREAMBLE_LEN];
        out.extend_from_slice(MAGIC);
        out.extend(explicit(0x0002, 0x0010, b"UI", &text(transfer_syntax)));
        out
    }

    fn dataset(element: fn(u16, u16, &[u8]) -> Vec<u8>) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(element(0x0008, 0x0060, &text("CT")));
        out.extend(element(0x0010, 0x0020, &text("PID-77")));
        out.extend(element(0x0020, 0x000D, &text("1.2.3.4")));
        out.extend(element(0x0028, 0x0010, &[0x00, 0x02]));
        out.extend(element(0x0028, 0x0011, &[0x00, 0x02]));
        out
    }

    fn explicit_element(group: u16, element: u16, value: &[u8]) -> Vec<u8> {
        // The VR for each tag this test uses.
        let vr: &[u8; 2] = match (group, element) {
            (0x0028, 0x0010 | 0x0011) => b"US",
            _ => b"LO",
        };
        explicit(group, element, vr, value)
    }

    fn explicit_file() -> Vec<u8> {
        let mut file = preamble(TS_EXPLICIT_LE);
        file.extend(dataset(explicit_element));
        file
    }

    fn implicit_file() -> Vec<u8> {
        let mut file = preamble(TS_IMPLICIT_LE);
        file.extend(dataset(implicit));
        file
    }

    #[test]
    fn explicit_little_endian_file_is_read() {
        let h = parse(&explicit_file()).expect("valid");
        assert_eq!(h.transfer_syntax, TransferSyntax::ExplicitLittleEndian);
        assert_eq!(h.patient_id, "PID-77");
        assert_eq!(h.study_uid, "1.2.3.4");
        assert_eq!(h.modality, "CT");
        assert_eq!((h.rows, h.columns), (512, 512));
    }

    #[test]
    fn implicit_little_endian_file_is_read() {
        let h = parse(&implicit_file()).expect("valid");
        assert_eq!(h.transfer_syntax, TransferSyntax::ImplicitLittleEndian);
        assert_eq!(h.patient_id, "PID-77");
        assert_eq!((h.rows, h.columns), (512, 512));
    }

    #[test]
    fn compressed_transfer_syntax_is_refused() {
        // JPEG Baseline.
        let mut file = preamble("1.2.840.10008.1.2.4.50");
        file.extend(dataset(explicit_element));
        assert!(matches!(
            parse(&file),
            Err(ParseError::UnsupportedTransferSyntax(_))
        ));
    }

    #[test]
    fn missing_preamble_is_refused() {
        assert_eq!(parse(&[0u8; 200]), Err(ParseError::NoPreamble));
    }

    #[test]
    fn missing_required_element_is_named() {
        let mut file = preamble(TS_EXPLICIT_LE);
        file.extend(explicit_element(0x0008, 0x0060, &text("CT")));
        assert_eq!(parse(&file), Err(ParseError::MissingElement("PatientID")));
    }

    #[test]
    fn element_longer_than_the_file_is_truncated() {
        let mut file = preamble(TS_EXPLICIT_LE);
        file.extend_from_slice(&0x0010u16.to_le_bytes());
        file.extend_from_slice(&0x0020u16.to_le_bytes());
        file.extend_from_slice(b"LO");
        file.extend_from_slice(&100u16.to_le_bytes());
        file.extend_from_slice(b"short");
        assert!(matches!(parse(&file), Err(ParseError::Truncated { .. })));
    }

    #[test]
    fn undefined_length_is_refused_rather_than_guessed() {
        let mut file = preamble(TS_IMPLICIT_LE);
        file.extend_from_slice(&0x0008u16.to_le_bytes());
        file.extend_from_slice(&0x0060u16.to_le_bytes());
        file.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            parse(&file),
            Err(ParseError::UndefinedLength { .. })
        ));
    }

    #[test]
    fn pixel_data_is_skipped_by_length() {
        let mut file = preamble(TS_EXPLICIT_LE);
        file.extend(dataset(explicit_element));
        // A large pixel-data element that must be skipped, not read.
        file.extend(explicit(0x7FE0, 0x0010, b"OW", &vec![0xAB; 4096]));
        let h = parse(&file).expect("valid");
        assert_eq!(h.patient_id, "PID-77");
    }

    #[test]
    fn bad_utf8_in_a_text_field_is_refused() {
        let mut file = preamble(TS_EXPLICIT_LE);
        file.extend(explicit_element(0x0010, 0x0020, &[0xff, 0xfe]));
        file.extend(dataset(explicit_element));
        assert_eq!(parse(&file), Err(ParseError::BadValue("PatientID")));
    }

    #[test]
    fn debug_output_redacts_identifiers() {
        let h = parse(&explicit_file()).expect("valid");
        let shown = format!("{h:?}");
        assert!(!shown.contains("PID-77"), "{shown}");
        assert!(!shown.contains("1.2.3.4"), "{shown}");
        assert!(shown.contains("CT"));
    }

    #[test]
    fn oversized_file_is_refused_before_parsing() {
        let big = vec![0u8; MAX_FILE_BYTES + 1];
        assert_eq!(
            parse(&big),
            Err(ParseError::TooLarge {
                len: MAX_FILE_BYTES + 1
            })
        );
    }

    /// A deterministic pseudo-random generator, so the test is repeatable.
    struct Lcg(u64);

    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.0 >> 33
        }

        fn below(&mut self, n: usize) -> usize {
            usize::try_from(self.next()).unwrap_or(0) % n.max(1)
        }
    }

    #[test]
    fn every_truncation_is_refused_without_panicking() {
        for file in [explicit_file(), implicit_file()] {
            for len in 0..file.len() {
                let _ = parse(&file[..len]);
            }
        }
    }

    #[test]
    fn mutated_input_never_panics() {
        let base = explicit_file();
        let mut rng = Lcg(0x00D1_C04D);
        for _ in 0..20_000 {
            let mut input = base.clone();
            for _ in 0..=rng.below(4) {
                let i = rng.below(input.len());
                input[i] = u8::try_from(rng.below(256)).unwrap_or(0);
            }
            let _ = parse(&input);
        }
    }
}
