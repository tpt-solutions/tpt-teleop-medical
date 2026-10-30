//! `HL7v2` ADT ingestion for tpt-teleop-medical (Phase 6).
//!
//! The parser accepts admit (ADT^A01), transfer (ADT^A02), and discharge
//! (ADT^A03) messages. It reads only three segments:
//!
//! * `MSH`: the message type (MSH-9) and the encoding characters (MSH-2).
//! * `PID`: the patient identifier (PID-3, first component).
//! * `PV1`: the patient location (PV1-3, first component). This is optional.
//!
//! Patient names, addresses, and dates of birth are not read. They are PHI, and
//! the bridge does not need them to place a patient.
//!
//! Input comes from the network, so every size is bounded and nothing panics on
//! malformed input. The limits are the `MAX_*` constants below. The HL7 parser
//! depends on nothing outside `std`. The FHIR parser ([`fhir`]) uses
//! `serde_json`.
//!
//! Escape sequences in HL7 (`\F\`, `\S\`, and so on) are not decoded. An
//! identifier that contains one is refused.

#![forbid(unsafe_code)]

pub mod audit;
pub mod fhir;

use core::fmt;

/// Largest message accepted, in bytes.
pub const MAX_MESSAGE_BYTES: usize = 65_536;
/// Most segments accepted in one message.
pub const MAX_SEGMENTS: usize = 64;
/// Most fields accepted in one segment.
pub const MAX_FIELDS: usize = 64;
/// Longest field accepted, in bytes.
pub const MAX_FIELD_BYTES: usize = 256;

const FIELD_SEP: char = '|';
const COMPONENT_SEP: char = '^';
const ESCAPE: char = '\\';
const ENCODING_CHARS: &str = "^~\\&";

/// What the message says happened to the patient.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdtKind {
    /// ADT^A01: the patient was admitted.
    Admit,
    /// ADT^A02: the patient was transferred.
    Transfer,
    /// ADT^A03: the patient was discharged.
    Discharge,
}

/// One parsed ADT message.
#[derive(Clone, PartialEq, Eq)]
pub struct AdtEvent {
    /// What happened.
    pub kind: AdtKind,
    /// The patient identifier, as sent. Treat it as PHI.
    pub patient_id: String,
    /// The patient's location, when the message has a PV1 segment.
    pub location: Option<String>,
}

/// `Debug` shows the kind and whether a location is present. It does not show
/// the patient identifier or the location.
impl fmt::Debug for AdtEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AdtEvent")
            .field("kind", &self.kind)
            .field("patient_id", &"<redacted>")
            .field("location", &self.location.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// Why a message was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// The message is empty, or has no segments.
    Empty,
    /// The message is longer than [`MAX_MESSAGE_BYTES`].
    TooLarge {
        /// The message length, in bytes.
        len: usize,
    },
    /// The message is not valid UTF-8.
    NotUtf8,
    /// The first segment is not a valid `MSH`.
    BadHeader,
    /// The message has more than [`MAX_SEGMENTS`] segments.
    TooManySegments,
    /// A segment has more than [`MAX_FIELDS`] fields.
    TooManyFields {
        /// The segment name.
        segment: &'static str,
    },
    /// A field is longer than [`MAX_FIELD_BYTES`].
    FieldTooLong {
        /// The segment name.
        segment: &'static str,
    },
    /// A required segment is missing.
    MissingSegment(&'static str),
    /// A required field is missing or empty.
    MissingField {
        /// The segment name.
        segment: &'static str,
        /// The field number.
        field: usize,
    },
    /// The identifier uses an escape sequence, which is not decoded.
    EscapeInIdentifier,
    /// The message is not an ADT message this bridge handles.
    UnsupportedMessage,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("message is empty"),
            Self::TooLarge { len } => write!(f, "message is {len} bytes, above the limit"),
            Self::NotUtf8 => f.write_str("message is not valid UTF-8"),
            Self::BadHeader => f.write_str("first segment is not a valid MSH"),
            Self::TooManySegments => f.write_str("message has too many segments"),
            Self::TooManyFields { segment } => write!(f, "{segment} has too many fields"),
            Self::FieldTooLong { segment } => write!(f, "{segment} has a field above the limit"),
            Self::MissingSegment(s) => write!(f, "required segment {s} is missing"),
            Self::MissingField { segment, field } => {
                write!(f, "{segment}-{field} is missing or empty")
            }
            Self::EscapeInIdentifier => f.write_str("identifier contains an escape sequence"),
            Self::UnsupportedMessage => f.write_str("message is not a supported ADT message"),
        }
    }
}

impl std::error::Error for ParseError {}

/// Parses one `HL7v2` ADT message.
///
/// # Errors
///
/// Returns the first problem found. Nothing is partly returned.
pub fn parse(message: &[u8]) -> Result<AdtEvent, ParseError> {
    if message.is_empty() {
        return Err(ParseError::Empty);
    }
    if message.len() > MAX_MESSAGE_BYTES {
        return Err(ParseError::TooLarge { len: message.len() });
    }
    let text = std::str::from_utf8(message).map_err(|_| ParseError::NotUtf8)?;

    let mut msh = None;
    let mut pid = None;
    let mut pv1 = None;
    let mut count = 0usize;
    for segment in text.split(['\r', '\n']).filter(|s| !s.is_empty()) {
        count += 1;
        if count > MAX_SEGMENTS {
            return Err(ParseError::TooManySegments);
        }
        // Map to a static name, so error values do not borrow the message.
        let name: &'static str = match segment.get(..3) {
            Some("MSH") => "MSH",
            Some("PID") => "PID",
            Some("PV1") => "PV1",
            _ => continue,
        };
        let slot = match name {
            "MSH" => &mut msh,
            "PID" => &mut pid,
            _ => &mut pv1,
        };
        // The first occurrence of each segment is the one used.
        if slot.is_none() {
            *slot = Some(split_fields(segment, name)?);
        }
    }

    let msh = msh.ok_or(ParseError::BadHeader)?;
    // MSH-2 is the encoding characters. They are fixed here, so a sender with
    // other separators is refused rather than misread.
    if msh.get(1).copied() != Some(ENCODING_CHARS) {
        return Err(ParseError::BadHeader);
    }
    let kind = adt_kind(&msh)?;

    let pid = pid.ok_or(ParseError::MissingSegment("PID"))?;
    let patient_id = first_component(&pid, "PID", 3)?;
    if patient_id.contains(ESCAPE) {
        return Err(ParseError::EscapeInIdentifier);
    }

    let location = match pv1 {
        Some(pv1) => {
            let loc = first_component(&pv1, "PV1", 3).ok();
            if loc.as_deref().is_some_and(|l| l.contains(ESCAPE)) {
                return Err(ParseError::EscapeInIdentifier);
            }
            loc
        }
        None => None,
    };

    Ok(AdtEvent {
        kind,
        patient_id,
        location,
    })
}

/// Splits a segment on the field separator. The field at index `n` is
/// `SEG-n`, and index 0 is the segment name. For MSH, index 1 is `MSH-2`, not
/// the separator itself.
fn split_fields<'a>(segment: &'a str, name: &'static str) -> Result<Vec<&'a str>, ParseError> {
    let mut fields = Vec::new();
    for field in segment.split(FIELD_SEP) {
        if fields.len() == MAX_FIELDS {
            return Err(ParseError::TooManyFields { segment: name });
        }
        if field.len() > MAX_FIELD_BYTES {
            return Err(ParseError::FieldTooLong { segment: name });
        }
        fields.push(field);
    }
    Ok(fields)
}

/// Returns the first component of field `index` (for example `PID-3.1`).
fn first_component(
    fields: &[&str],
    segment: &'static str,
    index: usize,
) -> Result<String, ParseError> {
    let component = fields
        .get(index)
        .and_then(|f| f.split(COMPONENT_SEP).next())
        .unwrap_or("");
    if component.is_empty() {
        return Err(ParseError::MissingField {
            segment,
            field: index,
        });
    }
    Ok(component.to_string())
}

/// Reads MSH-9 and maps the trigger event to a kind.
fn adt_kind(msh: &[&str]) -> Result<AdtKind, ParseError> {
    let message_type = msh.get(8).copied().unwrap_or("");
    let mut parts = message_type.split(COMPONENT_SEP);
    if parts.next() != Some("ADT") {
        return Err(ParseError::UnsupportedMessage);
    }
    match parts.next() {
        Some("A01") => Ok(AdtKind::Admit),
        Some("A02") => Ok(AdtKind::Transfer),
        Some("A03") => Ok(AdtKind::Discharge),
        _ => Err(ParseError::UnsupportedMessage),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(trigger: &str, pid_id: &str, pv1: Option<&str>) -> Vec<u8> {
        let mut segments = vec![
            format!("MSH|^~\\&|HIS|HOSP|LIS|HOSP|202601011200||ADT^{trigger}|MSG001|P|2.5"),
            "EVN|A01|202601011200".to_string(),
            format!("PID|1||{pid_id}^^^HOSP^MR||DOE^JANE|||F"),
        ];
        if let Some(pv1) = pv1 {
            segments.push(pv1.to_string());
        }
        segments.join("\r").into_bytes()
    }

    fn admit() -> Vec<u8> {
        message("A01", "MRN12345", Some("PV1|1|I|WARD4^401^A"))
    }

    #[test]
    fn admit_message_is_parsed() {
        let ev = parse(&admit()).expect("valid");
        assert_eq!(ev.kind, AdtKind::Admit);
        assert_eq!(ev.patient_id, "MRN12345");
        assert_eq!(ev.location.as_deref(), Some("WARD4"));
    }

    #[test]
    fn transfer_and_discharge_are_recognised() {
        let t = parse(&message("A02", "MRN1", Some("PV1|1|I|ICU^1"))).expect("valid");
        assert_eq!(t.kind, AdtKind::Transfer);
        let d = parse(&message("A03", "MRN1", Some("PV1|1|I|ICU^1"))).expect("valid");
        assert_eq!(d.kind, AdtKind::Discharge);
    }

    #[test]
    fn location_is_optional() {
        let ev = parse(&message("A01", "MRN1", None)).expect("valid");
        assert_eq!(ev.location, None);
    }

    #[test]
    fn other_triggers_are_unsupported() {
        assert_eq!(
            parse(&message("A08", "MRN1", None)),
            Err(ParseError::UnsupportedMessage)
        );
        assert_eq!(
            parse(b"MSH|^~\\&|A|B|C|D|t||ORU^R01|1|P|2.5\rPID|1||MRN1"),
            Err(ParseError::UnsupportedMessage)
        );
    }

    #[test]
    fn message_without_msh_is_refused() {
        assert_eq!(parse(b"PID|1||MRN1"), Err(ParseError::BadHeader));
    }

    #[test]
    fn unusual_encoding_characters_are_refused_not_misread() {
        assert_eq!(
            parse(b"MSH|#~\\&|A|B|C|D|t||ADT^A01|1|P|2.5\rPID|1||MRN1"),
            Err(ParseError::BadHeader)
        );
    }

    #[test]
    fn missing_pid_is_refused() {
        let bytes = b"MSH|^~\\&|A|B|C|D|t||ADT^A01|1|P|2.5\rPV1|1|I|WARD";
        assert_eq!(parse(bytes), Err(ParseError::MissingSegment("PID")));
    }

    #[test]
    fn empty_patient_identifier_is_refused() {
        assert_eq!(
            parse(&message("A01", "", None)),
            Err(ParseError::MissingField {
                segment: "PID",
                field: 3
            })
        );
    }

    #[test]
    fn escape_in_identifier_is_refused() {
        assert_eq!(
            parse(&message("A01", "MRN\\F\\1", None)),
            Err(ParseError::EscapeInIdentifier)
        );
    }

    #[test]
    fn oversized_message_is_refused_before_parsing() {
        let big = vec![b'A'; MAX_MESSAGE_BYTES + 1];
        assert_eq!(
            parse(&big),
            Err(ParseError::TooLarge {
                len: MAX_MESSAGE_BYTES + 1
            })
        );
    }

    #[test]
    fn too_many_segments_are_refused() {
        let mut segments = vec!["MSH|^~\\&|A|B|C|D|t||ADT^A01|1|P|2.5".to_string()];
        segments.extend((0..MAX_SEGMENTS).map(|_| "ZZZ|x".to_string()));
        let bytes = segments.join("\r").into_bytes();
        assert_eq!(parse(&bytes), Err(ParseError::TooManySegments));
    }

    #[test]
    fn overlong_field_is_refused() {
        let long = "9".repeat(MAX_FIELD_BYTES + 1);
        let bytes = format!("MSH|^~\\&|A|B|C|D|t||ADT^A01|1|P|2.5\rPID|1||{long}");
        assert_eq!(
            parse(bytes.as_bytes()),
            Err(ParseError::FieldTooLong { segment: "PID" })
        );
    }

    #[test]
    fn non_utf8_is_refused() {
        assert_eq!(parse(&[0xff, 0xfe, 0x00]), Err(ParseError::NotUtf8));
    }

    #[test]
    fn empty_input_is_refused() {
        assert_eq!(parse(b""), Err(ParseError::Empty));
    }

    #[test]
    fn debug_output_never_shows_identifiers() {
        let ev = parse(&admit()).expect("valid");
        let shown = format!("{ev:?}");
        assert!(!shown.contains("MRN12345"), "{shown}");
        assert!(!shown.contains("WARD4"), "{shown}");
        assert!(shown.contains("Admit"));
    }

    #[test]
    fn name_and_other_phi_fields_are_not_read() {
        let ev = parse(&admit()).expect("valid");
        let shown = format!("{ev:?}");
        assert!(!shown.contains("DOE"));
        assert!(!shown.contains("JANE"));
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

    /// Mutates a valid message and random bytes, and checks that parsing never
    /// panics. This is a repeatable stand-in for coverage-guided fuzzing, which
    /// needs a nightly toolchain. It does not replace that fuzzing.
    #[test]
    fn mutated_and_random_input_never_panics() {
        let base = admit();
        let mut rng = Lcg(0x5eed);
        for _ in 0..20_000 {
            let mut input = base.clone();
            for _ in 0..=rng.below(4) {
                match rng.below(3) {
                    0 if !input.is_empty() => {
                        let i = rng.below(input.len());
                        input[i] = u8::try_from(rng.below(256)).unwrap_or(b'?');
                    }
                    1 => {
                        let i = rng.below(input.len() + 1);
                        input.insert(i, u8::try_from(rng.below(256)).unwrap_or(b'|'));
                    }
                    _ if !input.is_empty() => {
                        let i = rng.below(input.len());
                        input.remove(i);
                    }
                    _ => {}
                }
            }
            let _ = parse(&input);
        }
        for _ in 0..5_000 {
            let len = rng.below(300);
            let input: Vec<u8> = (0..len)
                .map(|_| u8::try_from(rng.below(256)).unwrap_or(0))
                .collect();
            let _ = parse(&input);
        }
    }
}
