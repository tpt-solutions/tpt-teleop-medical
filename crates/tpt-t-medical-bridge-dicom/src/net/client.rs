//! The service class user (SCU) side of a DICOM association: C-ECHO, C-FIND,
//! C-GET and C-STORE, over any byte stream.
//!
//! [`Association::connect`] offers one presentation context per abstract syntax
//! asked for, each with the implicit and explicit little endian transfer syntaxes.
//! The peer accepts or refuses each. Operations on a refused syntax return
//! [`NetError::NoContext`].
//!
//! Retrieval is C-GET, not C-MOVE. C-MOVE needs a second association from the
//! peer back to a listener on this side. C-GET sends the instances over the
//! same association, so it needs no inbound connection.

use std::collections::VecDeque;
use std::fmt;
use std::io::{self, Read, Write};

use crate::net::dataset::{DatasetError, Element, decode_implicit, encode_implicit};
use crate::net::dimse::{
    C_ECHO_RQ, C_ECHO_RSP, C_FIND_RQ, C_FIND_RSP, C_GET_RQ, C_GET_RSP, C_STORE_RQ, C_STORE_RSP,
    Command, CommandError, STATUS_PENDING, STATUS_SUCCESS, STUDY_ROOT_FIND, STUDY_ROOT_GET,
    VERIFICATION,
};
use crate::net::pdu::{self, Context, Pdu, PduError, Pdv, TS_EXPLICIT_LE, TS_IMPLICIT_LE};

/// Largest command or data set accepted from a peer, in bytes.
pub const MAX_MESSAGE_BYTES: usize = 256 * 1024 * 1024;

/// Why a DICOM operation failed.
#[derive(Debug)]
pub enum NetError {
    /// The stream failed.
    Io(io::Error),
    /// The peer sent a PDU this side cannot accept.
    Pdu(PduError),
    /// The peer sent a command this side cannot decode.
    Command(CommandError),
    /// The peer refused the association.
    Rejected {
        /// The reason code from the rejection.
        reason: u8,
    },
    /// The peer aborted the association.
    Aborted,
    /// The peer returned a failure status.
    Status(u16),
    /// No accepted presentation context covers the operation.
    NoContext(String),
    /// The conversation went out of order.
    Protocol(&'static str),
    /// A message is larger than [`MAX_MESSAGE_BYTES`].
    TooLarge,
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "network error: {e}"),
            Self::Pdu(e) => write!(f, "{e}"),
            Self::Command(e) => write!(f, "{e}"),
            Self::Rejected { reason } => write!(f, "association rejected (reason {reason})"),
            Self::Aborted => f.write_str("association aborted by the peer"),
            Self::Status(s) => write!(f, "peer returned failure status {s:#06x}"),
            Self::NoContext(uid) => write!(f, "no accepted presentation context for {uid}"),
            Self::Protocol(what) => write!(f, "protocol error: {what}"),
            Self::TooLarge => f.write_str("message is above the size limit"),
        }
    }
}

impl std::error::Error for NetError {}

impl From<io::Error> for NetError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<PduError> for NetError {
    fn from(e: PduError) -> Self {
        Self::Pdu(e)
    }
}

impl From<DatasetError> for NetError {
    fn from(e: DatasetError) -> Self {
        Self::Command(CommandError::Dataset(e))
    }
}

impl From<CommandError> for NetError {
    fn from(e: CommandError) -> Self {
        Self::Command(e)
    }
}

/// One instance returned by a retrieve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retrieved {
    /// The SOP instance UID, from the store request.
    pub sop_instance: String,
    /// The data set, encoded in the transfer syntax the peer chose.
    pub dataset: Vec<u8>,
    /// That transfer syntax.
    pub transfer_syntax: String,
}

/// A negotiated presentation context.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Negotiated {
    id: u8,
    abstract_syntax: String,
    transfer_syntax: String,
}

/// An open association, from this side as the SCU.
pub struct Association<T: Read + Write> {
    io: T,
    contexts: Vec<Negotiated>,
    message_id: u16,
    pending: VecDeque<Pdv>,
}

impl<T: Read + Write> fmt::Debug for Association<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Association")
            .field("contexts", &self.contexts)
            .finish_non_exhaustive()
    }
}

impl<T: Read + Write> Association<T> {
    /// Opens an association over `io`, offering each abstract syntax.
    ///
    /// # Errors
    ///
    /// Returns [`NetError::Rejected`] if the peer refuses, or an I/O or protocol
    /// error if the exchange fails.
    pub fn connect(
        mut io: T,
        calling: &str,
        called: &str,
        abstract_syntaxes: &[&str],
    ) -> Result<Self, NetError> {
        let offered: Vec<Context> = abstract_syntaxes
            .iter()
            .enumerate()
            .map(|(i, syntax)| Context {
                id: u8::try_from(2 * i + 1).unwrap_or(u8::MAX),
                abstract_syntax: (*syntax).to_string(),
                transfer_syntaxes: vec![TS_IMPLICIT_LE.to_string(), TS_EXPLICIT_LE.to_string()],
                result: 0,
            })
            .collect();
        pdu::write_pdu(
            &mut io,
            &Pdu::AssociateRq {
                called: called.to_string(),
                calling: calling.to_string(),
                contexts: offered.clone(),
            },
        )?;
        let contexts = match pdu::read_pdu(&mut io)? {
            Pdu::AssociateAc { contexts } => contexts
                .into_iter()
                .filter(|c| c.result == 0)
                .filter_map(|c| {
                    let offer = offered.iter().find(|o| o.id == c.id)?;
                    Some(Negotiated {
                        id: c.id,
                        abstract_syntax: offer.abstract_syntax.clone(),
                        transfer_syntax: c.transfer_syntaxes.first()?.clone(),
                    })
                })
                .collect(),
            Pdu::AssociateRj { reason, .. } => return Err(NetError::Rejected { reason }),
            Pdu::Abort { .. } => return Err(NetError::Aborted),
            _ => return Err(NetError::Protocol("expected an association response")),
        };
        Ok(Self {
            io,
            contexts,
            message_id: 1,
            pending: VecDeque::new(),
        })
    }

    /// Returns the accepted transfer syntax for an abstract syntax, if the peer
    /// accepted it.
    #[must_use]
    pub fn transfer_syntax(&self, abstract_syntax: &str) -> Option<&str> {
        self.context(abstract_syntax)
            .ok()
            .map(|c| c.transfer_syntax.as_str())
    }

    /// Verifies that the peer answers C-ECHO. Returns its status.
    ///
    /// # Errors
    ///
    /// Returns an error if the exchange fails or the response is not a C-ECHO
    /// response.
    pub fn echo(&mut self) -> Result<u16, NetError> {
        let ctx = self.context(VERIFICATION)?.id;
        let rq = self.next_request(C_ECHO_RQ, VERIFICATION, false);
        self.send_command(ctx, &rq, &[])?;
        let (_, rsp, _) = self.recv_command()?;
        expect_field(&rsp, C_ECHO_RSP)?;
        Ok(rsp.status)
    }

    /// Asks the peer for the studies matching `query`. Returns each match as its
    /// decoded elements.
    ///
    /// # Errors
    ///
    /// Returns [`NetError::Status`] for a failed query, or an error if the
    /// exchange breaks down.
    pub fn find(&mut self, query: &[((u16, u16), Vec<u8>)]) -> Result<Vec<Vec<Element>>, NetError> {
        let ctx = self.context(STUDY_ROOT_FIND)?;
        let (ctx_id, ts) = (ctx.id, ctx.transfer_syntax.clone());
        let rq = self.next_request(C_FIND_RQ, STUDY_ROOT_FIND, true);
        // Find identifiers are sent in the transfer syntax the peer accepted.
        let dataset = if ts == TS_IMPLICIT_LE {
            encode_implicit(query)
        } else {
            return Err(NetError::Protocol(
                "only implicit VR little endian queries are sent",
            ));
        };
        self.send_command(ctx_id, &rq, &dataset)?;
        let mut matches = Vec::new();
        loop {
            let (_, rsp, data) = self.recv_command()?;
            expect_field(&rsp, C_FIND_RSP)?;
            match rsp.status {
                STATUS_PENDING => {
                    let data = data.ok_or(NetError::Protocol("pending match has no data set"))?;
                    matches.push(decode_implicit(&data)?);
                }
                STATUS_SUCCESS => return Ok(matches),
                other => return Err(NetError::Status(other)),
            }
        }
    }

    /// Retrieves the instances matching `query` with C-GET. Each instance comes
    /// back over this association as a C-STORE sub-operation, and is acknowledged
    /// as it arrives.
    ///
    /// # Errors
    ///
    /// Returns [`NetError::Status`] for a failed retrieve, or an error if the
    /// exchange breaks down.
    pub fn get(&mut self, query: &[((u16, u16), Vec<u8>)]) -> Result<Vec<Retrieved>, NetError> {
        let ctx_id = self.context(STUDY_ROOT_GET)?.id;
        let rq = self.next_request(C_GET_RQ, STUDY_ROOT_GET, true);
        self.send_command(ctx_id, &rq, &encode_implicit(query))?;
        let mut retrieved = Vec::new();
        loop {
            let (ctx, cmd, data) = self.recv_command()?;
            match cmd.field {
                C_STORE_RQ => {
                    let dataset =
                        data.ok_or(NetError::Protocol("store request has no data set"))?;
                    let transfer_syntax = self
                        .contexts
                        .iter()
                        .find(|c| c.id == ctx)
                        .map(|c| c.transfer_syntax.clone())
                        .ok_or(NetError::Protocol("store on an unknown context"))?;
                    retrieved.push(Retrieved {
                        sop_instance: cmd.affected_sop_instance.clone(),
                        dataset,
                        transfer_syntax,
                    });
                    let rsp = Command::response(&cmd, STATUS_SUCCESS);
                    self.send_command(ctx, &rsp, &[])?;
                }
                C_GET_RSP => match cmd.status {
                    STATUS_PENDING => {}
                    STATUS_SUCCESS => return Ok(retrieved),
                    other => return Err(NetError::Status(other)),
                },
                _ => return Err(NetError::Protocol("unexpected command during retrieve")),
            }
        }
    }

    /// Sends one data set to the peer with C-STORE. The data set must already be
    /// encoded in the transfer syntax the peer accepted for `sop_class`.
    ///
    /// # Errors
    ///
    /// Returns [`NetError::NoContext`] if the peer did not accept the class, or
    /// [`NetError::Status`] if the store fails.
    pub fn store(
        &mut self,
        sop_class: &str,
        sop_instance: &str,
        dataset: &[u8],
    ) -> Result<(), NetError> {
        let ctx_id = self.context(sop_class)?.id;
        let mut rq = self.next_request(C_STORE_RQ, sop_class, true);
        rq.affected_sop_instance = sop_instance.to_string();
        self.send_command(ctx_id, &rq, dataset)?;
        let (_, rsp, _) = self.recv_command()?;
        expect_field(&rsp, C_STORE_RSP)?;
        if rsp.status == STATUS_SUCCESS {
            Ok(())
        } else {
            Err(NetError::Status(rsp.status))
        }
    }

    /// Ends the association cleanly.
    ///
    /// # Errors
    ///
    /// Returns an error if the release exchange fails.
    pub fn release(mut self) -> Result<(), NetError> {
        pdu::write_pdu(&mut self.io, &Pdu::ReleaseRq)?;
        match pdu::read_pdu(&mut self.io)? {
            Pdu::ReleaseRp => Ok(()),
            Pdu::Abort { .. } => Err(NetError::Aborted),
            _ => Err(NetError::Protocol("expected a release response")),
        }
    }

    fn context(&self, abstract_syntax: &str) -> Result<&Negotiated, NetError> {
        self.contexts
            .iter()
            .find(|c| c.abstract_syntax == abstract_syntax)
            .ok_or_else(|| NetError::NoContext(abstract_syntax.to_string()))
    }

    fn next_request(&mut self, field: u16, sop_class: &str, has_dataset: bool) -> Command {
        let id = self.message_id;
        self.message_id = self.message_id.wrapping_add(1);
        Command::request(field, id, sop_class, has_dataset)
    }

    /// Sends a command, then its data set if it has one, on context `ctx`.
    fn send_command(&mut self, ctx: u8, command: &Command, dataset: &[u8]) -> Result<(), NetError> {
        let mut pdvs = pdu::fragment(ctx, true, &command.encode());
        if command.has_dataset {
            pdvs.extend(pdu::fragment(ctx, false, dataset));
        }
        pdu::write_pdu(&mut self.io, &Pdu::Data(pdvs))?;
        Ok(())
    }

    /// Reads one command and its data set, if it has one.
    fn recv_command(&mut self) -> Result<(u8, Command, Option<Vec<u8>>), NetError> {
        let (ctx, command_bytes) = self.recv_message(true)?;
        let command = Command::decode(&command_bytes)?;
        let dataset = if command.has_dataset {
            Some(self.recv_message(false)?.1)
        } else {
            None
        };
        Ok((ctx, command, dataset))
    }

    /// Collects the fragments of the next command (`command == true`) or data set
    /// message, across PDUs.
    fn recv_message(&mut self, command: bool) -> Result<(u8, Vec<u8>), NetError> {
        let mut bytes = Vec::new();
        let mut ctx = None;
        loop {
            let pdv = self.next_pdv()?;
            if pdv.command != command {
                // A data fragment arriving while a command is wanted, or the other
                // way round, means the peer is out of step.
                return Err(NetError::Protocol("message fragments arrived out of order"));
            }
            ctx.get_or_insert(pdv.context_id);
            if bytes.len() + pdv.data.len() > MAX_MESSAGE_BYTES {
                return Err(NetError::TooLarge);
            }
            bytes.extend_from_slice(&pdv.data);
            if pdv.last {
                return Ok((ctx.unwrap_or(0), bytes));
            }
        }
    }

    fn next_pdv(&mut self) -> Result<Pdv, NetError> {
        loop {
            if let Some(pdv) = self.pending.pop_front() {
                return Ok(pdv);
            }
            match pdu::read_pdu(&mut self.io)? {
                Pdu::Data(pdvs) => self.pending.extend(pdvs),
                Pdu::Abort { .. } => return Err(NetError::Aborted),
                _ => return Err(NetError::Protocol("expected data during a service")),
            }
        }
    }
}

fn expect_field(cmd: &Command, field: u16) -> Result<(), NetError> {
    if cmd.field == field {
        Ok(())
    } else {
        Err(NetError::Protocol("response is not the expected kind"))
    }
}
