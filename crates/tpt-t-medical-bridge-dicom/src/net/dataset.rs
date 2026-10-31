//! Data-element encoding for DICOM: Implicit and Explicit VR, little endian
//! (PS3.5 §7).
//!
//! Implicit VR carries no value representation, so a decoder reads the bytes
//! by tag. The command sets use it. Explicit VR is used for the datasets this
//! crate writes, so each element carries its VR.
//!
//! Elements must be in ascending tag order, and [`encode_implicit`] and
//! [`encode_explicit`] sort them, so a caller cannot produce an out-of-order
//! dataset by accident.

use core::fmt;

/// A DICOM tag, as (group, element).
pub type Tag = (u16, u16);

/// A decoded element. `vr` is `None` for an Implicit VR element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    /// The tag.
    pub tag: Tag,
    /// The value representation, if the encoding carried one.
    pub vr: Option<[u8; 2]>,
    /// The raw value bytes, without padding.
    pub value: Vec<u8>,
}

impl Element {
    /// The value as text, with the trailing padding removed.
    #[must_use]
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.value)
            .trim_end_matches(['\0', ' '])
            .to_string()
    }

    /// The value as an unsigned 16-bit integer, if it has the right length.
    #[must_use]
    pub fn u16(&self) -> Option<u16> {
        <[u8; 2]>::try_from(self.value.as_slice())
            .ok()
            .map(u16::from_le_bytes)
    }
}

/// Why a dataset could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatasetError {
    /// The bytes end inside an element.
    Truncated,
    /// An element length is undefined, or too large for this decoder.
    BadLength,
    /// An Explicit VR element has a value representation that is not known.
    BadVr,
}

impl fmt::Display for DatasetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => f.write_str("dataset is truncated"),
            Self::BadLength => f.write_str("element length is not usable"),
            Self::BadVr => f.write_str("unknown value representation"),
        }
    }
}

impl std::error::Error for DatasetError {}

/// Largest element value accepted, in bytes. Anything larger comes from an
/// untrusted peer and is refused.
pub const MAX_VALUE_BYTES: usize = 64 * 1024 * 1024;

/// VRs that use the 2-byte length form in Explicit VR.
const SHORT_LENGTH_VRS: [&[u8; 2]; 21] = [
    b"AE", b"AS", b"AT", b"CS", b"DA", b"DS", b"DT", b"FL", b"FD", b"IS", b"LO", b"LT", b"PN",
    b"SH", b"SL", b"SS", b"ST", b"TM", b"UI", b"UL", b"US",
];

/// Whether an Explicit VR uses the 2-byte length (`true`) or the 4-byte length
/// after two reserved bytes (`false`).
fn is_short(vr: [u8; 2]) -> bool {
    SHORT_LENGTH_VRS.contains(&&vr)
}

/// Whether `vr` is a value representation this crate writes.
fn known_vr(vr: [u8; 2]) -> bool {
    is_short(vr) || matches!(&vr, b"OB" | b"OW" | b"OF" | b"SQ" | b"UN" | b"UT")
}

/// Encodes elements as Implicit VR Little Endian, sorted by tag.
#[must_use]
pub fn encode_implicit(elements: &[(Tag, Vec<u8>)]) -> Vec<u8> {
    let mut sorted: Vec<&(Tag, Vec<u8>)> = elements.iter().collect();
    sorted.sort_by_key(|(tag, _)| *tag);
    let mut out = Vec::new();
    for (tag, value) in sorted {
        out.extend_from_slice(&tag.0.to_le_bytes());
        out.extend_from_slice(&tag.1.to_le_bytes());
        out.extend_from_slice(&padded_len(value).to_le_bytes());
        out.extend_from_slice(value);
        if value.len() % 2 == 1 {
            out.push(0);
        }
    }
    out
}

/// Encodes elements as Explicit VR Little Endian, sorted by tag. Each element is
/// given as `(tag, vr, value)`. Values are padded to an even length, with a
/// space for text and a zero for UI and binary values.
#[must_use]
pub fn encode_explicit(elements: &[(Tag, [u8; 2], Vec<u8>)]) -> Vec<u8> {
    let mut sorted: Vec<&(Tag, [u8; 2], Vec<u8>)> = elements.iter().collect();
    sorted.sort_by_key(|(tag, _, _)| *tag);
    let mut out = Vec::new();
    for (tag, vr, value) in sorted {
        out.extend_from_slice(&tag.0.to_le_bytes());
        out.extend_from_slice(&tag.1.to_le_bytes());
        out.extend_from_slice(vr);
        let mut bytes = value.clone();
        if bytes.len() % 2 == 1 {
            bytes.push(if matches!(vr, b"UI" | b"OB" | b"OW" | b"OF" | b"UN") {
                0
            } else {
                b' '
            });
        }
        if is_short(*vr) {
            let len = u16::try_from(bytes.len()).unwrap_or(u16::MAX);
            out.extend_from_slice(&len.to_le_bytes());
        } else {
            out.extend_from_slice(&[0, 0]);
            let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
            out.extend_from_slice(&len.to_le_bytes());
        }
        out.extend_from_slice(&bytes);
    }
    out
}

/// Encodes one sequence item, with a defined length, around `content`.
#[must_use]
pub fn sequence_item(content: &[u8]) -> Vec<u8> {
    let mut out = vec![0xFE, 0xFF, 0x00, 0xE0];
    out.extend_from_slice(
        &u32::try_from(content.len())
            .unwrap_or(u32::MAX)
            .to_le_bytes(),
    );
    out.extend_from_slice(content);
    out
}

/// Encodes an Explicit VR sequence element with defined length around `items`.
#[must_use]
pub fn sequence_element(tag: Tag, items: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = items.concat();
    let mut out = Vec::new();
    out.extend_from_slice(&tag.0.to_le_bytes());
    out.extend_from_slice(&tag.1.to_le_bytes());
    out.extend_from_slice(b"SQ");
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&u32::try_from(body.len()).unwrap_or(u32::MAX).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

fn padded_len(value: &[u8]) -> u32 {
    u32::try_from(value.len() + value.len() % 2).unwrap_or(u32::MAX)
}

/// Decodes Implicit VR Little Endian bytes. Every element is kept as raw bytes,
/// because a flat decode does not know the VRs.
///
/// # Errors
///
/// Returns an error if the bytes end inside an element, or a length is undefined
/// or too large.
pub fn decode_implicit(bytes: &[u8]) -> Result<Vec<Element>, DatasetError> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        let head = bytes.get(pos..pos + 8).ok_or(DatasetError::Truncated)?;
        let tag = (
            u16::from_le_bytes([head[0], head[1]]),
            u16::from_le_bytes([head[2], head[3]]),
        );
        let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]);
        pos += 8;
        let value = read_value(bytes, pos, len)?;
        pos += value.len() + usize::from(len % 2 == 1);
        out.push(Element {
            tag,
            vr: None,
            value,
        });
    }
    Ok(out)
}

/// Decodes Explicit VR Little Endian bytes. Sequence values are kept raw, so a
/// caller that needs them can decode each item.
///
/// # Errors
///
/// Returns an error for a truncated element, an undefined length, an
/// unknown VR, or a length above [`MAX_VALUE_BYTES`].
pub fn decode_explicit(bytes: &[u8]) -> Result<Vec<Element>, DatasetError> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        let head = bytes.get(pos..pos + 8).ok_or(DatasetError::Truncated)?;
        let tag = (
            u16::from_le_bytes([head[0], head[1]]),
            u16::from_le_bytes([head[2], head[3]]),
        );
        let vr = [head[4], head[5]];
        if !known_vr(vr) {
            return Err(DatasetError::BadVr);
        }
        let (len, value_start) = if is_short(vr) {
            (u32::from(u16::from_le_bytes([head[6], head[7]])), pos + 8)
        } else {
            let long = bytes
                .get(pos + 8..pos + 12)
                .ok_or(DatasetError::Truncated)?;
            (
                u32::from_le_bytes([long[0], long[1], long[2], long[3]]),
                pos + 12,
            )
        };
        let value = read_value(bytes, value_start, len)?;
        pos = value_start + value.len() + usize::from(len % 2 == 1);
        out.push(Element {
            tag,
            vr: Some(vr),
            value,
        });
    }
    Ok(out)
}

/// Splits a sequence value, as produced by [`sequence_element`], into its items.
///
/// # Errors
///
/// Returns an error if an item header is missing or truncated, or if an item is
/// not a defined-length item.
pub fn sequence_items(value: &[u8]) -> Result<Vec<&[u8]>, DatasetError> {
    let mut items = Vec::new();
    let mut pos = 0;
    while pos < value.len() {
        let head = value.get(pos..pos + 8).ok_or(DatasetError::Truncated)?;
        if head[..4] != [0xFE, 0xFF, 0x00, 0xE0] {
            return Err(DatasetError::BadLength);
        }
        let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        let item = value
            .get(pos + 8..pos + 8 + len)
            .ok_or(DatasetError::Truncated)?;
        items.push(item);
        pos += 8 + len;
    }
    Ok(items)
}

/// Reads `len` bytes at `pos`, refusing undefined and oversized lengths.
fn read_value(bytes: &[u8], pos: usize, len: u32) -> Result<Vec<u8>, DatasetError> {
    if len == u32::MAX {
        return Err(DatasetError::BadLength);
    }
    let len = len as usize;
    if len > MAX_VALUE_BYTES {
        return Err(DatasetError::BadLength);
    }
    // A pad byte may follow an odd-length value. It is read only if present, so
    // a value at the very end of the bytes still decodes.
    bytes
        .get(pos..pos + len)
        .map(<[u8]>::to_vec)
        .ok_or(DatasetError::Truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implicit_elements_are_sorted_and_round_trip() {
        let bytes = encode_implicit(&[
            ((0x0010, 0x0020), b"MRN-42".to_vec()),
            ((0x0008, 0x0060), b"CT".to_vec()),
        ]);
        let decoded = decode_implicit(&bytes).expect("decode");
        assert_eq!(decoded[0].tag, (0x0008, 0x0060));
        assert_eq!(decoded[0].text(), "CT");
        assert_eq!(decoded[1].text(), "MRN-42");
        assert_eq!(decoded[0].vr, None);
    }

    #[test]
    fn explicit_short_and_long_forms_round_trip() {
        let bytes = encode_explicit(&[
            ((0x0008, 0x0016), *b"UI", b"1.2.3".to_vec()),
            ((0x0028, 0x0010), *b"US", 512_u16.to_le_bytes().to_vec()),
            ((0x7FE0, 0x0010), *b"OB", vec![1, 2, 3]),
        ]);
        let decoded = decode_explicit(&bytes).expect("decode");
        assert_eq!(decoded[0].text(), "1.2.3");
        assert_eq!(decoded[1].u16(), Some(512));
        assert_eq!(decoded[2].value, vec![1, 2, 3, 0]);
        assert_eq!(decoded[2].vr, Some(*b"OB"));
    }

    #[test]
    fn odd_text_is_padded_to_even_length() {
        let bytes = encode_explicit(&[((0x0010, 0x0010), *b"PN", b"DOE^J".to_vec())]);
        assert_eq!(bytes.len() % 2, 0);
        assert_eq!(decode_explicit(&bytes).expect("decode")[0].text(), "DOE^J");
    }

    #[test]
    fn sequences_nest_and_split_into_items() {
        let inner = encode_explicit(&[((0x0008, 0x0100), *b"SH", b"121070".to_vec())]);
        let seq = sequence_element(
            (0x0040, 0xA043),
            &[sequence_item(&inner), sequence_item(&inner)],
        );
        let decoded = decode_explicit(&seq).expect("decode");
        assert_eq!(decoded[0].vr, Some(*b"SQ"));
        let items = sequence_items(&decoded[0].value).expect("items");
        assert_eq!(items.len(), 2);
        assert_eq!(decode_explicit(items[0]).expect("item")[0].text(), "121070");
    }

    #[test]
    fn truncation_and_undefined_lengths_are_refused() {
        let mut bytes = encode_implicit(&[((0x0010, 0x0020), b"MRN-42".to_vec())]);
        bytes.truncate(bytes.len() - 2);
        assert_eq!(decode_implicit(&bytes), Err(DatasetError::Truncated));
        let undefined = [0x08, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF];
        assert_eq!(decode_implicit(&undefined), Err(DatasetError::BadLength));
    }

    #[test]
    fn an_unknown_vr_is_refused() {
        let bytes = [0x08, 0x00, 0x16, 0x00, b'Z', b'Z', 0x00, 0x00];
        assert_eq!(decode_explicit(&bytes), Err(DatasetError::BadVr));
    }
}
