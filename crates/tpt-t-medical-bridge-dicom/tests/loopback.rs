//! The DICOM client exercised over real TCP, against a small service class
//! provider (SCP) on loopback. The SCP supports C-ECHO, C-FIND, C-GET and C-STORE
//! for the classes listed below, and refuses everything else.

use std::collections::VecDeque;
use std::io;
use std::net::{TcpListener, TcpStream};
use std::thread;

use tpt_t_medical_bridge_dicom::net::client::{Association, NetError};
use tpt_t_medical_bridge_dicom::net::dataset::{
    Element, decode_implicit, encode_explicit, encode_implicit,
};
use tpt_t_medical_bridge_dicom::net::dimse::{
    BASIC_TEXT_SR, C_ECHO_RQ, C_FIND_RQ, C_GET_RQ, C_STORE_RQ, C_STORE_RSP, CT_IMAGE_STORAGE,
    Command, MR_IMAGE_STORAGE, STATUS_PENDING, STATUS_SUCCESS, STUDY_ROOT_FIND, STUDY_ROOT_GET,
    VERIFICATION,
};
use tpt_t_medical_bridge_dicom::net::pdu::{
    self, Context, Pdu, Pdv, TS_EXPLICIT_LE, TS_IMPLICIT_LE,
};
use tpt_t_medical_bridge_dicom::sr::{Code, Report, export_basic_text_sr};

const CALLED: &str = "PACS";
const SUPPORTED: [&str; 5] = [
    VERIFICATION,
    STUDY_ROOT_FIND,
    STUDY_ROOT_GET,
    CT_IMAGE_STORAGE,
    BASIC_TEXT_SR,
];

/// A minimal SCP over one TCP connection.
struct Scp {
    stream: TcpStream,
    pending: VecDeque<Pdv>,
    /// Abstract syntax per context id, for storage decisions.
    classes: Vec<(u8, String)>,
    /// The instances the SCP has been sent by a store, as (class, instance).
    stored: Vec<(String, String)>,
}

fn released() -> io::Error {
    io::Error::other("released")
}

impl Scp {
    fn pdv(&mut self) -> io::Result<Pdv> {
        loop {
            if let Some(p) = self.pending.pop_front() {
                return Ok(p);
            }
            match pdu::read_pdu(&mut self.stream)? {
                Pdu::Data(v) => self.pending.extend(v),
                Pdu::ReleaseRq => {
                    pdu::write_pdu(&mut self.stream, &Pdu::ReleaseRp)?;
                    return Err(released());
                }
                other => return Err(io::Error::other(format!("unexpected {other:?}"))),
            }
        }
    }

    /// Reads one message: command or data, all fragments.
    fn message(&mut self, command: bool) -> io::Result<(u8, Vec<u8>)> {
        let mut bytes = Vec::new();
        loop {
            let p = self.pdv()?;
            bytes.extend_from_slice(&p.data);
            if p.last {
                assert_eq!(p.command, command, "fragment kind");
                return Ok((p.context_id, bytes));
            }
        }
    }

    fn request(&mut self) -> io::Result<(u8, Command, Option<Vec<u8>>)> {
        let (ctx, cmd_bytes) = self.message(true)?;
        let cmd = Command::decode(&cmd_bytes).map_err(io::Error::other)?;
        let data = if cmd.has_dataset {
            Some(self.message(false)?.1)
        } else {
            None
        };
        Ok((ctx, cmd, data))
    }

    fn send(&mut self, ctx: u8, cmd: &Command, data: &[u8]) -> io::Result<()> {
        let mut pdvs = pdu::fragment(ctx, true, &cmd.encode());
        if cmd.has_dataset {
            pdvs.extend(pdu::fragment(ctx, false, data));
        }
        pdu::write_pdu(&mut self.stream, &Pdu::Data(pdvs))
    }

    fn class_for(&self, ctx: u8) -> String {
        self.classes
            .iter()
            .find(|(id, _)| *id == ctx)
            .map(|(_, c)| c.clone())
            .unwrap_or_default()
    }

    /// Runs the association until the peer releases it.
    fn run(&mut self) -> io::Result<()> {
        loop {
            let (ctx, cmd, data) = match self.request() {
                Ok(v) => v,
                Err(e) if e.to_string() == "released" => return Ok(()),
                Err(e) => return Err(e),
            };
            match cmd.field {
                C_ECHO_RQ => {
                    let rsp = Command::response(&cmd, STATUS_SUCCESS);
                    self.send(ctx, &rsp, &[])?;
                }
                C_FIND_RQ => {
                    let query =
                        decode_implicit(&data.unwrap_or_default()).map_err(io::Error::other)?;
                    let patient = query
                        .iter()
                        .find(|e| e.tag == (0x0010, 0x0020))
                        .map(Element::text)
                        .unwrap_or_default();
                    let mut pending = Command::response(&cmd, STATUS_PENDING);
                    pending.has_dataset = true;
                    let row = encode_implicit(&[
                        ((0x0010, 0x0020), patient.into_bytes()),
                        ((0x0020, 0x000D), b"1.2.3".to_vec()),
                        ((0x0008, 0x0060), b"CT".to_vec()),
                    ]);
                    self.send(ctx, &pending, &row)?;
                    let done = Command::response(&cmd, STATUS_SUCCESS);
                    self.send(ctx, &done, &[])?;
                }
                C_GET_RQ => {
                    // Send one CT instance back as a C-STORE sub-operation, in the
                    // storage context the client negotiated.
                    let storage_ctx = self
                        .classes
                        .iter()
                        .find(|(_, c)| c == CT_IMAGE_STORAGE)
                        .map_or(1, |(id, _)| *id);
                    let mut store = Command::request(C_STORE_RQ, 100, CT_IMAGE_STORAGE, true);
                    store.affected_sop_instance = "1.2.3.4.5".into();
                    let instance = encode_explicit(&[
                        (
                            (0x0008, 0x0016),
                            *b"UI",
                            CT_IMAGE_STORAGE.as_bytes().to_vec(),
                        ),
                        ((0x0008, 0x0018), *b"UI", b"1.2.3.4.5".to_vec()),
                        ((0x0008, 0x0060), *b"CS", b"CT".to_vec()),
                    ]);
                    self.send(storage_ctx, &store, &instance)?;
                    let (_, rsp_bytes) = self.message(true)?;
                    let rsp = Command::decode(&rsp_bytes).map_err(io::Error::other)?;
                    if rsp.field != C_STORE_RSP || rsp.status != STATUS_SUCCESS {
                        return Err(io::Error::other("store not acknowledged"));
                    }
                    let done = Command::response(&cmd, STATUS_SUCCESS);
                    self.send(ctx, &done, &[])?;
                }
                C_STORE_RQ => {
                    let class = self.class_for(ctx);
                    if class != cmd.affected_sop_class {
                        return Err(io::Error::other("store on the wrong class"));
                    }
                    self.stored.push((
                        cmd.affected_sop_class.clone(),
                        cmd.affected_sop_instance.clone(),
                    ));
                    let rsp = Command::response(&cmd, STATUS_SUCCESS);
                    self.send(ctx, &rsp, &[])?;
                }
                other => {
                    return Err(io::Error::other(format!(
                        "unsupported command {other:#06x}"
                    )));
                }
            }
        }
    }
}

/// Accepts one association. Refuses a called title other than [`CALLED`] with a
/// rejection, and refuses unsupported abstract syntaxes per context.
fn serve(listener: &TcpListener) -> io::Result<()> {
    let (mut stream, _) = listener.accept()?;
    let Pdu::AssociateRq {
        called, contexts, ..
    } = pdu::read_pdu(&mut stream)?
    else {
        return Err(io::Error::other("expected an association request"));
    };
    if called != CALLED {
        pdu::write_pdu(
            &mut stream,
            &Pdu::AssociateRj {
                result: 1,
                source: 1,
                reason: 7,
            },
        )?;
        return Ok(());
    }
    let mut accepted = Vec::new();
    let mut classes = Vec::new();
    for ctx in &contexts {
        let supported = SUPPORTED.contains(&ctx.abstract_syntax.as_str());
        // Storage classes take the explicit syntax, everything else implicit.
        let chosen =
            if ctx.abstract_syntax == CT_IMAGE_STORAGE || ctx.abstract_syntax == BASIC_TEXT_SR {
                TS_EXPLICIT_LE
            } else {
                TS_IMPLICIT_LE
            };
        accepted.push(Context {
            id: ctx.id,
            abstract_syntax: String::new(),
            transfer_syntaxes: vec![chosen.to_string()],
            result: if supported { 0 } else { 3 },
        });
        if supported {
            classes.push((ctx.id, ctx.abstract_syntax.clone()));
        }
    }
    pdu::write_pdu(&mut stream, &Pdu::AssociateAc { contexts: accepted })?;
    let mut scp = Scp {
        stream,
        pending: VecDeque::new(),
        classes,
        stored: Vec::new(),
    };
    scp.run()
}

/// Starts an SCP on loopback and returns its address and the thread.
fn start_scp() -> (std::net::SocketAddr, thread::JoinHandle<io::Result<()>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    (addr, thread::spawn(move || serve(&listener)))
}

fn connect(addr: std::net::SocketAddr, classes: &[&str]) -> Association<TcpStream> {
    let stream = TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .expect("timeout");
    Association::connect(stream, "TPT-MEDICAL", CALLED, classes).expect("associate")
}

#[test]
fn c_echo_succeeds_over_tcp() {
    let (addr, server) = start_scp();
    let mut assoc = connect(addr, &[VERIFICATION]);
    assert_eq!(assoc.echo().expect("echo"), STATUS_SUCCESS);
    assoc.release().expect("release");
    server.join().expect("join").expect("scp");
}

#[test]
fn c_find_returns_the_matching_study() {
    let (addr, server) = start_scp();
    let mut assoc = connect(addr, &[STUDY_ROOT_FIND]);
    let matches = assoc
        .find(&[((0x0010, 0x0020), b"MRN-42".to_vec())])
        .expect("find");
    assert_eq!(matches.len(), 1);
    let patient = matches[0]
        .iter()
        .find(|e| e.tag == (0x0010, 0x0020))
        .expect("patient");
    assert_eq!(patient.text(), "MRN-42");
    assoc.release().expect("release");
    server.join().expect("join").expect("scp");
}

#[test]
fn c_get_retrieves_the_instance_with_its_transfer_syntax() {
    let (addr, server) = start_scp();
    let mut assoc = connect(addr, &[STUDY_ROOT_GET, CT_IMAGE_STORAGE]);
    let retrieved = assoc
        .get(&[((0x0010, 0x0020), b"MRN-42".to_vec())])
        .expect("get");
    assert_eq!(retrieved.len(), 1);
    assert_eq!(retrieved[0].sop_instance, "1.2.3.4.5");
    assert_eq!(retrieved[0].transfer_syntax, TS_EXPLICIT_LE);
    assoc.release().expect("release");
    server.join().expect("join").expect("scp");
}

#[test]
fn an_exported_report_is_stored_on_the_peer() {
    let report = Report {
        patient_id: "MRN-42".into(),
        study_uid: "1.2.3".into(),
        series_uid: "1.2.3.1".into(),
        sop_instance_uid: "1.2.3.1.1".into(),
        date: "20260612".into(),
        time: "093015".into(),
        title: Code {
            value: "18748-4".into(),
            scheme: "LN".into(),
            meaning: "Diagnostic imaging report".into(),
        },
        findings: vec!["Port sites clean.".into()],
    };
    let bytes = export_basic_text_sr(&report).expect("export");
    // Skip the preamble and the meta group, which the peer does not need in the
    // dataset: C-STORE sends the data set alone.
    let meta_len = {
        let len = u32::from_le_bytes([bytes[140], bytes[141], bytes[142], bytes[143]]) as usize;
        12 + len
    };
    let dataset = &bytes[132 + meta_len..];

    let (addr, server) = start_scp();
    let mut assoc = connect(addr, &[BASIC_TEXT_SR]);
    assoc
        .store(BASIC_TEXT_SR, "1.2.3.1.1", dataset)
        .expect("store");
    assoc.release().expect("release");
    server.join().expect("join").expect("scp");
}

#[test]
fn a_class_the_peer_refuses_is_reported_as_no_context() {
    let (addr, server) = start_scp();
    let mut assoc = connect(addr, &[MR_IMAGE_STORAGE, VERIFICATION]);
    let err = assoc
        .store(MR_IMAGE_STORAGE, "1.2.3", &[])
        .expect_err("refused");
    assert!(matches!(err, NetError::NoContext(_)), "{err}");
    assert_eq!(assoc.echo().expect("echo still works"), STATUS_SUCCESS);
    assoc.release().expect("release");
    server.join().expect("join").expect("scp");
}

#[test]
fn a_rejected_association_is_an_error_not_a_hang() {
    let (addr, server) = start_scp();
    let stream = TcpStream::connect(addr).expect("connect");
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .expect("timeout");
    let err = Association::connect(stream, "TPT-MEDICAL", "WRONG-PEER", &[VERIFICATION])
        .expect_err("rejected");
    assert!(matches!(err, NetError::Rejected { reason: 7 }), "{err}");
    server.join().expect("join").expect("scp");
}
