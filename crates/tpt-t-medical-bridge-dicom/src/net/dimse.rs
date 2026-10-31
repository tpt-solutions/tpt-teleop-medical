//! DIMSE command messages: the requests and responses of C-ECHO, C-FIND,
//! C-GET and C-STORE (PS3.7).
//!
//! A command is a group-0000 Implicit VR element set. It says what operation is
//! asked for and whether a data set follows. The data set, if any, goes in a
//! separate message on the same presentation context.

use crate::net::dataset::{DatasetError, Element, decode_implicit, encode_implicit};

/// Command field: C-STORE request.
pub const C_STORE_RQ: u16 = 0x0001;
/// Command field: C-GET request.
pub const C_GET_RQ: u16 = 0x0010;
/// Command field: C-FIND request.
pub const C_FIND_RQ: u16 = 0x0020;
/// Command field: C-ECHO request.
pub const C_ECHO_RQ: u16 = 0x0030;
/// Command field: C-STORE response.
pub const C_STORE_RSP: u16 = 0x8001;
/// Command field: C-GET response.
pub const C_GET_RSP: u16 = 0x8010;
/// Command field: C-FIND response.
pub const C_FIND_RSP: u16 = 0x8020;
/// Command field: C-ECHO response.
pub const C_ECHO_RSP: u16 = 0x8030;

/// Status: success.
pub const STATUS_SUCCESS: u16 = 0x0000;
/// Status: a match is pending, more follow.
pub const STATUS_PENDING: u16 = 0xFF00;
/// Status: the operation was cancelled.
pub const STATUS_CANCEL: u16 = 0xFE00;

/// Command data set type: no data set follows.
pub const NO_DATASET: u16 = 0x0101;
/// Command data set type: a data set follows.
pub const HAS_DATASET: u16 = 0x0000;

/// Abstract syntax (SOP class) of the Verification service, for C-ECHO.
pub const VERIFICATION: &str = "1.2.840.10008.1.1";
/// Study Root Query/Retrieve, FIND.
pub const STUDY_ROOT_FIND: &str = "1.2.840.10008.5.1.4.1.2.2.1";
/// Study Root Query/Retrieve, GET.
pub const STUDY_ROOT_GET: &str = "1.2.840.10008.5.1.4.1.2.2.3";
/// CT Image Storage.
pub const CT_IMAGE_STORAGE: &str = "1.2.840.10008.5.1.4.1.1.2";
/// MR Image Storage.
pub const MR_IMAGE_STORAGE: &str = "1.2.840.10008.5.1.4.1.1.4";
/// Basic Text Structured Report Storage.
pub const BASIC_TEXT_SR: &str = "1.2.840.10008.5.1.4.1.1.88.11";

/// A command set. Only the elements this crate uses are modelled, and the
/// message ID is the number that links a response to its request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// The command field, such as [`C_FIND_RQ`].
    pub field: u16,
    /// The message ID of the request, echoed in the response.
    pub message_id: u16,
    /// The SOP class the command is about.
    pub affected_sop_class: String,
    /// The SOP instance, for a store, or empty.
    pub affected_sop_instance: String,
    /// Whether a data set follows this command.
    pub has_dataset: bool,
    /// The status, in a response.
    pub status: u16,
    /// Extra numeric sub-operation counts, as (element, value), for C-GET responses.
    pub counts: Vec<(u16, u16)>,
}

/// Why a command could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandError {
    /// The command set is not valid Implicit VR.
    Dataset(DatasetError),
    /// A required element is missing.
    Missing(&'static str),
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dataset(e) => write!(f, "{e}"),
            Self::Missing(what) => write!(f, "command is missing {what}"),
        }
    }
}

impl std::error::Error for CommandError {}

impl From<DatasetError> for CommandError {
    fn from(e: DatasetError) -> Self {
        Self::Dataset(e)
    }
}

impl Command {
    /// A request for `field`, with no status and no counts.
    #[must_use]
    pub fn request(field: u16, message_id: u16, sop_class: &str, has_dataset: bool) -> Self {
        Self {
            field,
            message_id,
            affected_sop_class: sop_class.to_string(),
            affected_sop_instance: String::new(),
            has_dataset,
            status: STATUS_SUCCESS,
            counts: Vec::new(),
        }
    }

    /// A response to a request, with the given status.
    #[must_use]
    pub fn response(request: &Self, status: u16) -> Self {
        Self {
            field: request.field | 0x8000,
            message_id: request.message_id,
            affected_sop_class: request.affected_sop_class.clone(),
            affected_sop_instance: request.affected_sop_instance.clone(),
            has_dataset: false,
            status,
            counts: Vec::new(),
        }
    }

    /// Encodes the command set, with its group length.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut elements: Vec<((u16, u16), Vec<u8>)> = Vec::new();
        elements.push(((0x0000, 0x0002), uid(&self.affected_sop_class)));
        elements.push(((0x0000, 0x0100), self.field.to_le_bytes().to_vec()));
        elements.push(((0x0000, 0x0110), self.message_id.to_le_bytes().to_vec()));
        if self.field & 0x8000 == 0 {
            elements.push(((0x0000, 0x0700), 0_u16.to_le_bytes().to_vec()));
        }
        let dataset_type = if self.has_dataset {
            HAS_DATASET
        } else {
            NO_DATASET
        };
        elements.push(((0x0000, 0x0800), dataset_type.to_le_bytes().to_vec()));
        if self.field & 0x8000 != 0 {
            elements.push(((0x0000, 0x0900), self.status.to_le_bytes().to_vec()));
        }
        if !self.affected_sop_instance.is_empty() {
            elements.push(((0x0000, 0x1000), uid(&self.affected_sop_instance)));
        }
        for (element, value) in &self.counts {
            elements.push(((0x0000, *element), value.to_le_bytes().to_vec()));
        }
        elements.sort_by_key(|(tag, _)| *tag);
        let body = encode_implicit(&elements);
        let group_len = u32::try_from(body.len()).unwrap_or(u32::MAX);
        let mut out = encode_implicit(&[((0x0000, 0x0000), group_len.to_le_bytes().to_vec())]);
        out.extend_from_slice(&body);
        out
    }

    /// Decodes a command set.
    ///
    /// # Errors
    ///
    /// Returns an error if the bytes are not Implicit VR, or if the command field
    /// or message ID is missing.
    pub fn decode(bytes: &[u8]) -> Result<Self, CommandError> {
        let elements = decode_implicit(bytes)?;
        let find = |element: u16| -> Option<&Element> {
            elements.iter().find(|e| e.tag == (0x0000, element))
        };
        let field = find(0x0100)
            .and_then(Element::u16)
            .ok_or(CommandError::Missing("CommandField"))?;
        let message_id = find(0x0110).and_then(Element::u16).unwrap_or(0);
        let has_dataset = find(0x0800).and_then(Element::u16).unwrap_or(NO_DATASET) != NO_DATASET;
        let status = find(0x0900)
            .and_then(Element::u16)
            .unwrap_or(STATUS_SUCCESS);
        let affected_sop_class = find(0x0002).map(Element::text).unwrap_or_default();
        let affected_sop_instance = find(0x1000).map(Element::text).unwrap_or_default();
        let counts = elements
            .iter()
            .filter(|e| e.tag.0 == 0x0000 && e.tag.1 >= 0x1020 && e.tag.1 <= 0x1023)
            .filter_map(|e| e.u16().map(|v| (e.tag.1, v)))
            .collect();
        Ok(Self {
            field,
            message_id,
            affected_sop_class,
            affected_sop_instance,
            has_dataset,
            status,
            counts,
        })
    }
}

/// Encodes a UID as its value bytes, padded to an even length with a zero.
fn uid(value: &str) -> Vec<u8> {
    let mut bytes = value.as_bytes().to_vec();
    if bytes.len() % 2 == 1 {
        bytes.push(0);
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_round_trips() {
        let rq = Command::request(C_FIND_RQ, 7, STUDY_ROOT_FIND, true);
        let back = Command::decode(&rq.encode()).expect("decode");
        assert_eq!(back.field, C_FIND_RQ);
        assert_eq!(back.message_id, 7);
        assert_eq!(back.affected_sop_class, STUDY_ROOT_FIND);
        assert!(back.has_dataset);
    }

    #[test]
    fn a_response_echoes_the_request_and_carries_the_status() {
        let rq = Command::request(C_ECHO_RQ, 42, VERIFICATION, false);
        let rsp = Command::response(&rq, STATUS_SUCCESS);
        let back = Command::decode(&rsp.encode()).expect("decode");
        assert_eq!(back.field, C_ECHO_RSP);
        assert_eq!(back.message_id, 42);
        assert!(!back.has_dataset);
        assert_eq!(back.status, STATUS_SUCCESS);
    }

    #[test]
    fn the_group_length_is_the_length_of_the_rest() {
        let bytes = Command::request(C_ECHO_RQ, 1, VERIFICATION, false).encode();
        // The first element is (0000,0000) with a 4-byte value: 12 bytes in all.
        let group_len = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
        assert_eq!(group_len, bytes.len() - 12);
    }

    #[test]
    fn counts_survive_a_round_trip() {
        let mut rsp = Command::response(
            &Command::request(C_GET_RQ, 3, STUDY_ROOT_GET, false),
            0xFF00,
        );
        rsp.counts = vec![(0x1020, 4), (0x1021, 1)];
        let back = Command::decode(&rsp.encode()).expect("decode");
        assert_eq!(back.counts, vec![(0x1020, 4), (0x1021, 1)]);
        assert_eq!(back.status, STATUS_PENDING);
    }

    #[test]
    fn a_command_without_a_field_is_refused() {
        let bytes = encode_implicit(&[((0x0000, 0x0110), 1_u16.to_le_bytes().to_vec())]);
        assert_eq!(
            Command::decode(&bytes),
            Err(CommandError::Missing("CommandField"))
        );
    }
}
