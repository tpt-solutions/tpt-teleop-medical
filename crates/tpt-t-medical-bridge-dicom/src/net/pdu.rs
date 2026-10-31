//! Protocol data units of the DICOM Upper Layer (PS3.8): association setup,
//! data transfer, release and abort, over a byte stream.
//!
//! Every length is checked before it is used, because the peer is untrusted. A
//! PDU above [`MAX_PDU_BYTES`] is refused.

use std::io::{self, Read, Write};

/// Largest PDU body accepted from a peer, in bytes.
pub const MAX_PDU_BYTES: usize = 1024 * 1024;
/// Largest data fragment this side sends in one PDV, in bytes. It is also the
/// maximum PDU length we advertise.
pub const FRAGMENT_BYTES: usize = 16_384;

/// The DICOM application context name.
pub const APPLICATION_CONTEXT: &str = "1.2.840.10008.3.1.1.1";
/// The implicit VR little endian transfer syntax.
pub const TS_IMPLICIT_LE: &str = "1.2.840.10008.1.2";
/// The explicit VR little endian transfer syntax.
pub const TS_EXPLICIT_LE: &str = "1.2.840.10008.1.2.1";
/// The implementation class UID this crate identifies itself with.
pub const IMPLEMENTATION_UID: &str = "1.2.826.0.1.3680043.10.1.1";

const PDU_ASSOCIATE_RQ: u8 = 0x01;
const PDU_ASSOCIATE_AC: u8 = 0x02;
const PDU_ASSOCIATE_RJ: u8 = 0x03;
const PDU_DATA: u8 = 0x04;
const PDU_RELEASE_RQ: u8 = 0x05;
const PDU_RELEASE_RP: u8 = 0x06;
const PDU_ABORT: u8 = 0x07;

/// Error for a malformed or unexpected PDU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PduError {
    /// The PDU is shorter than its own structure needs.
    Truncated,
    /// The PDU type is not one this crate knows.
    UnknownType(u8),
    /// The PDU is larger than [`MAX_PDU_BYTES`].
    TooLarge(usize),
    /// A field has a value the protocol does not allow.
    Malformed(&'static str),
}

impl std::fmt::Display for PduError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated => f.write_str("PDU is truncated"),
            Self::UnknownType(t) => write!(f, "unknown PDU type {t:#04x}"),
            Self::TooLarge(n) => write!(f, "PDU of {n} bytes is above the limit"),
            Self::Malformed(what) => write!(f, "malformed PDU: {what}"),
        }
    }
}

impl std::error::Error for PduError {}

/// One proposed or accepted presentation context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    /// The odd context identifier, 1 to 255.
    pub id: u8,
    /// The abstract syntax (SOP class) for a request. Empty for an acceptance.
    pub abstract_syntax: String,
    /// The transfer syntaxes proposed, or the one accepted.
    pub transfer_syntaxes: Vec<String>,
    /// The result code from an acceptance. Zero is accepted.
    pub result: u8,
}

/// A decoded PDU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pdu {
    /// Association request, with the calling and called titles.
    AssociateRq {
        /// The called application entity title.
        called: String,
        /// The calling application entity title.
        calling: String,
        /// The proposed presentation contexts.
        contexts: Vec<Context>,
    },
    /// Association accepted, with the contexts the peer accepted.
    AssociateAc {
        /// The contexts with their results.
        contexts: Vec<Context>,
    },
    /// Association rejected.
    AssociateRj {
        /// The result code.
        result: u8,
        /// The source code.
        source: u8,
        /// The reason code.
        reason: u8,
    },
    /// Data fragments.
    Data(Vec<Pdv>),
    /// Release request.
    ReleaseRq,
    /// Release response.
    ReleaseRp,
    /// Abort.
    Abort {
        /// The source of the abort.
        source: u8,
        /// The reason code.
        reason: u8,
    },
}

/// One presentation data value: a fragment of a command or data set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pdv {
    /// The presentation context the fragment belongs to.
    pub context_id: u8,
    /// True for a command fragment, false for a data set fragment.
    pub command: bool,
    /// True for the last fragment of a message.
    pub last: bool,
    /// The fragment bytes.
    pub data: Vec<u8>,
}

/// The maximum PDU length as the 4-byte value the protocol carries.
fn fragment_len() -> u32 {
    u32::try_from(FRAGMENT_BYTES).unwrap_or(u32::MAX)
}

fn pad_ae(title: &str) -> [u8; 16] {
    let mut out = [b' '; 16];
    for (slot, b) in out.iter_mut().zip(title.bytes().take(16)) {
        *slot = b;
    }
    out
}

fn push_item(out: &mut Vec<u8>, item_type: u8, body: &[u8]) {
    out.push(item_type);
    out.push(0);
    out.extend_from_slice(&u16::try_from(body.len()).unwrap_or(u16::MAX).to_be_bytes());
    out.extend_from_slice(body);
}

fn uid_item(item_type: u8, uid: &str) -> Vec<u8> {
    let mut out = Vec::new();
    push_item(&mut out, item_type, uid.as_bytes());
    out
}

impl Pdu {
    /// Encodes the PDU, header included.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let (kind, body) = match self {
            Self::AssociateRq {
                called,
                calling,
                contexts,
            } => (
                PDU_ASSOCIATE_RQ,
                encode_associate_rq(called, calling, contexts),
            ),
            Self::AssociateAc { contexts } => (PDU_ASSOCIATE_AC, encode_associate_ac(contexts)),
            Self::AssociateRj {
                result,
                source,
                reason,
            } => (PDU_ASSOCIATE_RJ, vec![0, *result, *source, *reason]),
            Self::Data(pdvs) => (PDU_DATA, encode_data(pdvs)),
            Self::ReleaseRq => (PDU_RELEASE_RQ, vec![0; 4]),
            Self::ReleaseRp => (PDU_RELEASE_RP, vec![0; 4]),
            Self::Abort { source, reason } => (PDU_ABORT, vec![0, 0, *source, *reason]),
        };
        let mut out = vec![kind, 0];
        out.extend_from_slice(&u32::try_from(body.len()).unwrap_or(u32::MAX).to_be_bytes());
        out.extend_from_slice(&body);
        out
    }

    /// Decodes a PDU body of the given type.
    ///
    /// # Errors
    ///
    /// Returns an error for a truncated body, an unknown type, or a field the
    /// protocol does not allow.
    pub fn decode(kind: u8, body: &[u8]) -> Result<Self, PduError> {
        match kind {
            PDU_ASSOCIATE_RQ => decode_associate_rq(body),
            PDU_ASSOCIATE_AC => decode_associate_ac(body),
            PDU_ASSOCIATE_RJ => {
                let b = body.get(..4).ok_or(PduError::Truncated)?;
                Ok(Self::AssociateRj {
                    result: b[1],
                    source: b[2],
                    reason: b[3],
                })
            }
            PDU_DATA => decode_data(body),
            PDU_RELEASE_RQ => Ok(Self::ReleaseRq),
            PDU_RELEASE_RP => Ok(Self::ReleaseRp),
            PDU_ABORT => {
                let b = body.get(..4).ok_or(PduError::Truncated)?;
                Ok(Self::Abort {
                    source: b[2],
                    reason: b[3],
                })
            }
            other => Err(PduError::UnknownType(other)),
        }
    }
}

fn encode_associate_rq(called: &str, calling: &str, contexts: &[Context]) -> Vec<u8> {
    let mut out = vec![0, 1, 0, 0];
    out.extend_from_slice(&pad_ae(called));
    out.extend_from_slice(&pad_ae(calling));
    out.extend_from_slice(&[0; 32]);
    push_item(&mut out, 0x10, APPLICATION_CONTEXT.as_bytes());
    for ctx in contexts {
        let mut body = vec![ctx.id, 0, 0, 0];
        body.extend(uid_item(0x30, &ctx.abstract_syntax));
        for ts in &ctx.transfer_syntaxes {
            body.extend(uid_item(0x40, ts));
        }
        push_item(&mut out, 0x20, &body);
    }
    let mut user = Vec::new();
    push_item(&mut user, 0x51, &fragment_len().to_be_bytes());
    user.extend(uid_item(0x52, IMPLEMENTATION_UID));
    push_item(&mut out, 0x50, &user);
    out
}

fn encode_associate_ac(contexts: &[Context]) -> Vec<u8> {
    let mut out = vec![0, 1, 0, 0];
    out.extend_from_slice(&[b' '; 16]);
    out.extend_from_slice(&[b' '; 16]);
    out.extend_from_slice(&[0; 32]);
    push_item(&mut out, 0x10, APPLICATION_CONTEXT.as_bytes());
    for ctx in contexts {
        let mut body = vec![ctx.id, 0, ctx.result, 0];
        for ts in &ctx.transfer_syntaxes {
            body.extend(uid_item(0x40, ts));
        }
        push_item(&mut out, 0x21, &body);
    }
    let mut user = Vec::new();
    push_item(&mut user, 0x51, &fragment_len().to_be_bytes());
    push_item(&mut out, 0x50, &user);
    out
}

fn encode_data(pdvs: &[Pdv]) -> Vec<u8> {
    let mut out = Vec::new();
    for pdv in pdvs {
        let len = u32::try_from(pdv.data.len() + 2).unwrap_or(u32::MAX);
        out.extend_from_slice(&len.to_be_bytes());
        out.push(pdv.context_id);
        let mut control = 0_u8;
        if pdv.command {
            control |= 0x01;
        }
        if pdv.last {
            control |= 0x02;
        }
        out.push(control);
        out.extend_from_slice(&pdv.data);
    }
    out
}

/// Reads the items of an association PDU body, after its fixed header.
fn items(body: &[u8]) -> Result<Vec<(u8, &[u8])>, PduError> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < body.len() {
        let head = body.get(pos..pos + 4).ok_or(PduError::Truncated)?;
        let len = usize::from(u16::from_be_bytes([head[2], head[3]]));
        let data = body
            .get(pos + 4..pos + 4 + len)
            .ok_or(PduError::Truncated)?;
        out.push((head[0], data));
        pos += 4 + len;
    }
    Ok(out)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim_end_matches(['\0', ' '])
        .to_string()
}

fn decode_associate_rq(body: &[u8]) -> Result<Pdu, PduError> {
    if body.len() < 68 {
        return Err(PduError::Truncated);
    }
    let called = text(&body[4..20]);
    let calling = text(&body[20..36]);
    let mut contexts = Vec::new();
    for (kind, data) in items(&body[68..])? {
        if kind == 0x20 {
            let (id, rest) = data.split_first().ok_or(PduError::Truncated)?;
            let mut ctx = Context {
                id: *id,
                abstract_syntax: String::new(),
                transfer_syntaxes: Vec::new(),
                result: 0,
            };
            // Three reserved bytes follow the id. Sub-items come after them.
            let subs = rest.get(3..).ok_or(PduError::Truncated)?;
            for (sub, value) in items(subs)? {
                match sub {
                    0x30 => ctx.abstract_syntax = text(value),
                    0x40 => ctx.transfer_syntaxes.push(text(value)),
                    _ => {}
                }
            }
            contexts.push(ctx);
        }
    }
    Ok(Pdu::AssociateRq {
        called,
        calling,
        contexts,
    })
}

fn decode_associate_ac(body: &[u8]) -> Result<Pdu, PduError> {
    if body.len() < 68 {
        return Err(PduError::Truncated);
    }
    let mut contexts = Vec::new();
    for (kind, data) in items(&body[68..])? {
        if kind == 0x21 {
            let [id, _, result, _, subs @ ..] = data else {
                return Err(PduError::Truncated);
            };
            let mut ctx = Context {
                id: *id,
                abstract_syntax: String::new(),
                transfer_syntaxes: Vec::new(),
                result: *result,
            };
            for (sub, value) in items(subs)? {
                if sub == 0x40 {
                    ctx.transfer_syntaxes.push(text(value));
                }
            }
            contexts.push(ctx);
        }
    }
    Ok(Pdu::AssociateAc { contexts })
}

fn decode_data(body: &[u8]) -> Result<Pdu, PduError> {
    let mut pdvs = Vec::new();
    let mut pos = 0;
    while pos < body.len() {
        let head = body.get(pos..pos + 4).ok_or(PduError::Truncated)?;
        let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        if len < 2 {
            return Err(PduError::Malformed("presentation data value too short"));
        }
        let pdv = body
            .get(pos + 4..pos + 4 + len)
            .ok_or(PduError::Truncated)?;
        let control = pdv[1];
        pdvs.push(Pdv {
            context_id: pdv[0],
            command: control & 0x01 != 0,
            last: control & 0x02 != 0,
            data: pdv[2..].to_vec(),
        });
        pos += 4 + len;
    }
    Ok(Pdu::Data(pdvs))
}

/// Reads one PDU from a stream.
///
/// # Errors
///
/// Returns an I/O error from the stream, or a [`PduError`] wrapped as
/// `InvalidData` for a malformed PDU or one above [`MAX_PDU_BYTES`].
pub fn read_pdu<R: Read>(r: &mut R) -> io::Result<Pdu> {
    let mut head = [0_u8; 6];
    r.read_exact(&mut head)?;
    let len = u32::from_be_bytes([head[2], head[3], head[4], head[5]]) as usize;
    if len > MAX_PDU_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            PduError::TooLarge(len),
        ));
    }
    let mut body = vec![0_u8; len];
    r.read_exact(&mut body)?;
    Pdu::decode(head[0], &body).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Writes one PDU to a stream.
///
/// # Errors
///
/// Returns any error from the stream.
pub fn write_pdu<W: Write>(w: &mut W, pdu: &Pdu) -> io::Result<()> {
    w.write_all(&pdu.encode())?;
    w.flush()
}

/// Splits a message into data PDVs of at most [`FRAGMENT_BYTES`] each. The last
/// fragment has its last flag set.
#[must_use]
pub fn fragment(context_id: u8, command: bool, data: &[u8]) -> Vec<Pdv> {
    let mut out: Vec<Pdv> = data
        .chunks(FRAGMENT_BYTES)
        .map(|c| Pdv {
            context_id,
            command,
            last: false,
            data: c.to_vec(),
        })
        .collect();
    if let Some(last) = out.last_mut() {
        last.last = true;
    } else {
        out.push(Pdv {
            context_id,
            command,
            last: true,
            data: Vec::new(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(id: u8, abstract_syntax: &str) -> Context {
        Context {
            id,
            abstract_syntax: abstract_syntax.into(),
            transfer_syntaxes: vec![TS_IMPLICIT_LE.into(), TS_EXPLICIT_LE.into()],
            result: 0,
        }
    }

    fn round_trip(pdu: &Pdu) -> Pdu {
        let bytes = pdu.encode();
        let mut slice: &[u8] = &bytes;
        read_pdu(&mut slice).expect("read")
    }

    #[test]
    fn associate_request_round_trips() {
        let rq = Pdu::AssociateRq {
            called: "PACS".into(),
            calling: "TPT-MEDICAL".into(),
            contexts: vec![
                ctx(1, "1.2.840.10008.1.1"),
                ctx(3, "1.2.840.10008.5.1.4.1.2.2.1"),
            ],
        };
        assert_eq!(round_trip(&rq), rq);
    }

    #[test]
    fn associate_accept_and_reject_round_trip() {
        let ac = Pdu::AssociateAc {
            contexts: vec![Context {
                id: 1,
                abstract_syntax: String::new(),
                transfer_syntaxes: vec![TS_EXPLICIT_LE.into()],
                result: 0,
            }],
        };
        assert_eq!(round_trip(&ac), ac);
        let rj = Pdu::AssociateRj {
            result: 1,
            source: 1,
            reason: 2,
        };
        assert_eq!(round_trip(&rj), rj);
    }

    #[test]
    fn data_pdvs_keep_their_flags() {
        let data = Pdu::Data(vec![
            Pdv {
                context_id: 3,
                command: true,
                last: true,
                data: vec![1, 2, 3],
            },
            Pdv {
                context_id: 3,
                command: false,
                last: false,
                data: vec![4],
            },
        ]);
        assert_eq!(round_trip(&data), data);
    }

    #[test]
    fn release_and_abort_round_trip() {
        assert_eq!(round_trip(&Pdu::ReleaseRq), Pdu::ReleaseRq);
        assert_eq!(round_trip(&Pdu::ReleaseRp), Pdu::ReleaseRp);
        let abort = Pdu::Abort {
            source: 2,
            reason: 0,
        };
        assert_eq!(round_trip(&abort), abort);
    }

    #[test]
    fn oversized_and_unknown_pdus_are_refused() {
        let mut huge = vec![PDU_DATA, 0];
        huge.extend_from_slice(
            &u32::try_from(MAX_PDU_BYTES + 1)
                .unwrap_or(u32::MAX)
                .to_be_bytes(),
        );
        let mut slice: &[u8] = &huge;
        assert!(read_pdu(&mut slice).is_err());
        assert_eq!(Pdu::decode(0x7F, &[]), Err(PduError::UnknownType(0x7F)));
    }

    #[test]
    fn fragmentation_splits_and_marks_the_last_piece() {
        let data = vec![9_u8; FRAGMENT_BYTES * 2 + 10];
        let pdvs = fragment(5, false, &data);
        assert_eq!(pdvs.len(), 3);
        assert!(pdvs.iter().all(|p| p.context_id == 5 && !p.command));
        assert!(!pdvs[0].last && !pdvs[1].last && pdvs[2].last);
        assert_eq!(pdvs.iter().map(|p| p.data.len()).sum::<usize>(), data.len());
        assert_eq!(fragment(1, true, &[]).len(), 1);
    }
}
