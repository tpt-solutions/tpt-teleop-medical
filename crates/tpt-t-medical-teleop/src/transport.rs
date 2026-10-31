//! The network transport between a surgeon and the arm (spec §4.6, §5.5).
//!
//! Two paths run between the units, and each has its own UDP socket:
//!
//! * the command path carries the operator's [`ControlCommand`]s to the arm,
//! * the feed path carries [`Feed`] telemetry from the arm back to the operator.
//!
//! Each path is set up first by a signed handshake (`tpt-t-sec`) on its own
//! socket. After that, every datagram is AEAD-sealed under the negotiated
//! session and framed by `tpt-t-link`. Plaintext never leaves the unit.
//!
//! The rules:
//!
//! * The operator initiates both handshakes, and accepts only the enrolled arm.
//! * The arm answers only enrolled operators, and only with the roles that may
//!   command it (`Operator` or `AiAgent`).
//! * A live session cannot be taken over. A new handshake is accepted only after
//!   the current session has been silent for the takeover window.
//! * Each side stamps a sequence number on what it sends, and the receiver
//!   refuses any number that does not rise. A recorded datagram cannot be
//!   replayed.
//! * Timestamps come from the sender's clock. One-way latency is meaningful only
//!   when both units share a clock, such as a PTP or GPS disciplined clock. The
//!   link monitors treat a clock fault as a lost link.
//!
//! Not yet: video and point-cloud frames, key rotation, and reliable
//! retransmission of commands.

use std::io;
use std::net::SocketAddr;
use std::thread;
use std::time::{Duration, Instant};

use tpt_t_core::ser::cmd::MAGIC_COMMAND;
use tpt_t_core::ser::{
    AlignedBuf, ControlCommand, TelemetryKind, TelemetryPacket, access_root, serialize_into,
};
use tpt_t_link::crc::command_crc;
use tpt_t_link::mux::{Inbound, MAX_DATAGRAM, MAX_PAYLOAD, RxBuffer, UdpMux, secure_inner};
use tpt_t_sec::cipher::CipherSuite;
use tpt_t_sec::identity::{ATTESTATION_WIRE_LEN, Attestation, DeviceIdentity};
use tpt_t_sec::link::{AAD_CONTROL, AAD_TELEMETRY};
use tpt_t_sec::rbac::Role;
use tpt_t_sec::session::{
    HandshakeInit, HandshakeResp, SecureSession, begin_handshake, finish_handshake,
    respond_handshake,
};
use tpt_t_sec::zerotrust::TrustStore;

/// Handshake message tags. They are the first byte of a handshake datagram.
const MSG_INIT: u8 = 1;
const MSG_RESP: u8 = 2;

/// How long one handshake attempt waits for its reply before it is sent again.
const ATTEMPT: Duration = Duration::from_millis(200);

/// Pause between polls of a socket that has nothing to read.
const POLL: Duration = Duration::from_millis(1);

/// Length of the ephemeral key echoed in a reply, which ties it to one attempt.
const ECHO_LEN: usize = 32;

/// Why a transport call failed.
#[derive(Debug)]
pub enum TransportError {
    /// The socket failed.
    Io(io::Error),
    /// The handshake did not complete in time.
    Timeout,
    /// The peer is not enrolled, or its role may not use this path.
    Untrusted,
    /// The path has no session yet.
    NotEstablished,
    /// A message could not be encoded, or a cryptographic step failed.
    Crypto,
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "socket error: {e}"),
            Self::Timeout => write!(f, "handshake timed out"),
            Self::Untrusted => write!(f, "peer is not trusted for this path"),
            Self::NotEstablished => write!(f, "path has no session yet"),
            Self::Crypto => write!(f, "message could not be encoded or sealed"),
        }
    }
}

impl std::error::Error for TransportError {}

impl From<io::Error> for TransportError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// One telemetry packet, as the operator sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Feed {
    /// The arm's sequence number for this packet.
    pub seq: u64,
    /// When the arm sent it, on the arm's clock, in nanoseconds.
    pub timestamp_ns: u64,
    /// Measured values: force (x, y, z), tool position (x, y, z), battery
    /// percent and fault flags. Unused slots are zero.
    pub values: [f32; 8],
}

/// Which way a path carries traffic. It fixes the AEAD domain and the frame tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Command,
    Feed,
}

impl Kind {
    fn aad(self) -> &'static [u8] {
        match self {
            Self::Command => AAD_CONTROL,
            Self::Feed => AAD_TELEMETRY,
        }
    }

    fn inner(self) -> u8 {
        match self {
            Self::Command => secure_inner::CONTROL,
            Self::Feed => secure_inner::TELEMETRY,
        }
    }
}

/// Only enrolled operators, and AI agents acting for them, may drive the arm.
fn may_command(role: Role) -> bool {
    matches!(role, Role::Operator | Role::AiAgent)
}

fn suite_byte(suite: CipherSuite) -> u8 {
    match suite {
        CipherSuite::Aes256Gcm => 1,
        CipherSuite::ChaCha20Poly1305 => 2,
    }
}

fn suite_from(byte: u8) -> Option<CipherSuite> {
    match byte {
        1 => Some(CipherSuite::Aes256Gcm),
        2 => Some(CipherSuite::ChaCha20Poly1305),
        _ => None,
    }
}

/// Encodes an init: tag, suite count, suites, attestation.
fn encode_init(init: &HandshakeInit) -> Vec<u8> {
    let count = u8::try_from(init.suites.len()).unwrap_or(u8::MAX);
    let mut out = vec![MSG_INIT, count];
    out.extend(init.suites.iter().map(|&s| suite_byte(s)));
    out.extend_from_slice(&init.attestation.to_bytes());
    out
}

fn decode_init(bytes: &[u8]) -> Option<HandshakeInit> {
    let (&tag, rest) = bytes.split_first()?;
    let (&count, rest) = rest.split_first()?;
    if tag != MSG_INIT || rest.len() != usize::from(count) + ATTESTATION_WIRE_LEN {
        return None;
    }
    let (suites, att) = rest.split_at(usize::from(count));
    Some(HandshakeInit {
        attestation: Attestation::from_bytes(att).ok()?,
        suites: suites
            .iter()
            .map(|&b| suite_from(b))
            .collect::<Option<Vec<_>>>()?,
    })
}

/// Encodes a reply: tag, chosen suite, echo of the init's ephemeral key,
/// attestation.
fn encode_resp(resp: &HandshakeResp, echo: &[u8; ECHO_LEN]) -> Vec<u8> {
    let mut out = vec![MSG_RESP, suite_byte(resp.suite)];
    out.extend_from_slice(echo);
    out.extend_from_slice(&resp.attestation.to_bytes());
    out
}

fn decode_resp(bytes: &[u8]) -> Option<(HandshakeResp, [u8; ECHO_LEN])> {
    if bytes.len() != 2 + ECHO_LEN + ATTESTATION_WIRE_LEN || bytes[0] != MSG_RESP {
        return None;
    }
    let suite = suite_from(bytes[1])?;
    let mut echo = [0u8; ECHO_LEN];
    echo.copy_from_slice(&bytes[2..2 + ECHO_LEN]);
    let attestation = Attestation::from_bytes(&bytes[2 + ECHO_LEN..]).ok()?;
    Some((HandshakeResp { attestation, suite }, echo))
}

/// One secured socket: the mux, the peer, and the session once one exists.
struct Path {
    kind: Kind,
    mux: UdpMux,
    rx: RxBuffer,
    session: Option<SecureSession>,
    peer: Option<SocketAddr>,
    /// When the last authenticated datagram arrived, on this unit's monotonic clock.
    last_rx: Option<Instant>,
    /// The highest sequence number accepted so far.
    last_seq: Option<u64>,
    /// Where the last opened datagram's plaintext is kept. Sized once, so the
    /// receive path does not allocate.
    plain: AlignedBuf,
}

impl Path {
    fn new(kind: Kind, mux: UdpMux) -> Self {
        let mut plain = AlignedBuf::new();
        plain.resize(MAX_PAYLOAD, 0);
        Self {
            kind,
            mux,
            rx: RxBuffer::new(),
            session: None,
            peer: None,
            last_rx: None,
            last_seq: None,
            plain,
        }
    }

    /// True while the session has received traffic within `silence`.
    fn is_live(&self, silence: Duration) -> bool {
        self.last_rx.is_some_and(|t| t.elapsed() < silence)
    }

    /// Frames an unencrypted handshake message and sends it to `dst`.
    fn send_plain(&mut self, bytes: &[u8], dst: SocketAddr) -> io::Result<()> {
        let mut buf = [0u8; MAX_DATAGRAM];
        let n = self.mux.write_ice_frame(bytes, &mut buf)?;
        self.mux.send_framed(dst, &buf[..n], 0)?;
        Ok(())
    }

    /// Returns the next handshake datagram, if one has arrived.
    fn poll_plain(&mut self) -> io::Result<Option<(Vec<u8>, SocketAddr)>> {
        match self.mux.recv_frame(&mut self.rx)? {
            Some(Ok(Inbound::Ice { payload, from })) => Ok(Some((payload.to_vec(), from))),
            _ => Ok(None),
        }
    }

    /// Seals `plaintext` under the session and sends it to the peer.
    fn send_sealed(&mut self, plaintext: &[u8], now_ns: u64) -> Result<(), TransportError> {
        let (Some(session), Some(dst)) = (self.session.as_ref(), self.peer) else {
            return Err(TransportError::NotEstablished);
        };
        let mut sealed = [0u8; MAX_PAYLOAD];
        let n = session
            .crypto()
            .seal_into(plaintext, self.kind.aad(), &mut sealed)
            .map_err(|_| TransportError::Crypto)?;
        let mut buf = [0u8; MAX_DATAGRAM];
        let len = self
            .mux
            .write_secure_frame(&sealed[..n], self.kind.inner(), &mut buf)?;
        self.mux.send_framed(dst, &buf[..len], now_ns)?;
        Ok(())
    }

    /// Opens the next authenticated datagram on this path. The plaintext is left
    /// in `self.plain[..n]`, and `n` is returned. Anything unauthenticated,
    /// malformed or on the wrong channel is dropped, and the call returns `None`
    /// for it.
    fn recv_open(&mut self) -> Result<Option<usize>, TransportError> {
        let Some(session) = self.session.as_ref() else {
            return Ok(None);
        };
        let Some(frame) = self.mux.recv_frame(&mut self.rx)? else {
            return Ok(None);
        };
        let Ok(Inbound::Secure { sealed, inner, .. }) = frame else {
            return Ok(None);
        };
        if inner != self.kind.inner() {
            return Ok(None);
        }
        match session
            .crypto()
            .open_into(sealed, self.kind.aad(), &mut self.plain[..])
        {
            Ok(n) => {
                self.last_rx = Some(Instant::now());
                Ok(Some(n))
            }
            Err(_) => Ok(None),
        }
    }

    /// Runs the initiator side of the handshake against `peer`, which must be
    /// the enrolled unit `expected`.
    fn connect(
        &mut self,
        identity: &DeviceIdentity,
        trust: &TrustStore,
        expected: u64,
        peer: SocketAddr,
        timeout: Duration,
    ) -> Result<(), TransportError> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let (init, pending) = begin_handshake(
                identity,
                &[CipherSuite::Aes256Gcm, CipherSuite::ChaCha20Poly1305],
            )
            .map_err(|_| TransportError::Crypto)?;
            let echo = init.attestation.eph_pub;
            self.send_plain(&encode_init(&init), peer)?;
            let attempt_end = (Instant::now() + ATTEMPT).min(deadline);
            while Instant::now() < attempt_end {
                let Some((bytes, _from)) = self.poll_plain()? else {
                    thread::sleep(POLL);
                    continue;
                };
                let Some((resp, reply_echo)) = decode_resp(&bytes) else {
                    continue;
                };
                // A reply to an earlier attempt, or a forgery, does not match this
                // attempt's ephemeral key.
                if reply_echo != echo {
                    continue;
                }
                trust
                    .verify(&resp.attestation)
                    .map_err(|_| TransportError::Untrusted)?;
                if resp.attestation.unit_id != expected {
                    return Err(TransportError::Untrusted);
                }
                let session = finish_handshake(identity, pending, &resp)
                    .map_err(|_| TransportError::Crypto)?;
                self.session = Some(session);
                self.peer = Some(peer);
                self.last_rx = None;
                return Ok(());
            }
        }
        Err(TransportError::Timeout)
    }

    /// Runs the responder side of the handshake. It waits for one acceptable
    /// init, answers it, and adopts the session. A session still in use is not
    /// taken over.
    fn accept(
        &mut self,
        identity: &DeviceIdentity,
        trust: &TrustStore,
        timeout: Duration,
        takeover_silence: Duration,
    ) -> Result<(), TransportError> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let Some((bytes, from)) = self.poll_plain()? else {
                thread::sleep(POLL);
                continue;
            };
            let Some(init) = decode_init(&bytes) else {
                continue;
            };
            let Ok(role) = trust.verify(&init.attestation) else {
                continue;
            };
            if !may_command(role) || self.is_live(takeover_silence) {
                continue;
            }
            let Ok((resp, session)) = respond_handshake(
                identity,
                &init,
                &[CipherSuite::Aes256Gcm, CipherSuite::ChaCha20Poly1305],
            ) else {
                continue;
            };
            self.send_plain(&encode_resp(&resp, &init.attestation.eph_pub), from)?;
            self.session = Some(session);
            self.peer = Some(from);
            self.last_rx = None;
            return Ok(());
        }
        Err(TransportError::Timeout)
    }
}

/// The surgeon's end of the link.
pub struct Operator {
    command: Path,
    feed: Path,
    next_seq: u64,
    scratch: AlignedBuf,
}

impl Operator {
    /// Sets up both paths to the arm. The arm must already be accepting.
    ///
    /// `now_ns` seeds the command sequence numbers. A restarted operator then
    /// continues above the numbers it used before, as long as its clock has moved
    /// on.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError::Timeout`] if the arm does not answer in time,
    /// [`TransportError::Untrusted`] if the arm is not the enrolled unit, or a
    /// socket or crypto error.
    #[allow(clippy::too_many_arguments)]
    pub fn connect(
        identity: &DeviceIdentity,
        trust: &TrustStore,
        arm_id: u64,
        arm_command: SocketAddr,
        arm_feed: SocketAddr,
        command: UdpMux,
        feed: UdpMux,
        now_ns: u64,
        timeout: Duration,
    ) -> Result<Self, TransportError> {
        let mut command_path = Path::new(Kind::Command, command);
        let mut feed_path = Path::new(Kind::Feed, feed);
        command_path.connect(identity, trust, arm_id, arm_command, timeout)?;
        feed_path.connect(identity, trust, arm_id, arm_feed, timeout)?;
        Ok(Self {
            command: command_path,
            feed: feed_path,
            next_seq: now_ns,
            scratch: sized_scratch(),
        })
    }

    /// Sends one command to the arm. The sequence number and the timestamp are
    /// set here, and the caller sets the axes and the mode.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError::NotEstablished`] before connect, or a socket or
    /// crypto error.
    pub fn send_command(
        &mut self,
        mut cmd: ControlCommand,
        now_ns: u64,
    ) -> Result<(), TransportError> {
        cmd.magic = MAGIC_COMMAND;
        cmd.seq = self.next_seq;
        cmd.timestamp_ns = now_ns;
        cmd.crc = command_crc(&cmd);
        self.next_seq += 1;
        let n = serialize_into(&cmd, &mut self.scratch).map_err(|_| TransportError::Crypto)?;
        self.command.send_sealed(&self.scratch[..n], now_ns)
    }

    /// Returns the next telemetry packet from the arm, if one has arrived. A
    /// packet that is not newer than the last one accepted is dropped.
    ///
    /// # Errors
    ///
    /// Returns a socket error, or [`TransportError::NotEstablished`] before connect.
    pub fn recv_feed(&mut self) -> Result<Option<Feed>, TransportError> {
        let Some(n) = self.feed.recv_open()? else {
            return Ok(None);
        };
        let Ok(arch) = access_root::<TelemetryPacket>(&self.feed.plain[..n]) else {
            return Ok(None);
        };
        let mut values = [0.0_f32; 8];
        for (dst, src) in values.iter_mut().zip(arch.values.iter()) {
            *dst = src.to_native();
        }
        let feed = Feed {
            seq: arch.seq.to_native(),
            timestamp_ns: arch.timestamp_ns.to_native(),
            values,
        };
        if self.feed.last_seq.is_some_and(|s| feed.seq <= s) {
            return Ok(None);
        }
        self.feed.last_seq = Some(feed.seq);
        Ok(Some(feed))
    }
}

/// The arm's end of the link.
pub struct Arm {
    command: Path,
    feed: Path,
    next_seq: u64,
    scratch: AlignedBuf,
}

impl Arm {
    /// Accepts the operator on both paths. Each path waits up to `timeout` for
    /// its handshake.
    ///
    /// A session that has received traffic within `takeover_silence` is not
    /// replaced. A second operator must wait for the first to go quiet.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError::Timeout`] if no acceptable operator connects.
    #[allow(clippy::too_many_arguments)]
    pub fn accept(
        identity: &DeviceIdentity,
        trust: &TrustStore,
        command: UdpMux,
        feed: UdpMux,
        now_ns: u64,
        timeout: Duration,
        takeover_silence: Duration,
    ) -> Result<Self, TransportError> {
        let mut command_path = Path::new(Kind::Command, command);
        let mut feed_path = Path::new(Kind::Feed, feed);
        command_path.accept(identity, trust, timeout, takeover_silence)?;
        feed_path.accept(identity, trust, timeout, takeover_silence)?;
        Ok(Self {
            command: command_path,
            feed: feed_path,
            next_seq: now_ns,
            scratch: sized_scratch(),
        })
    }

    /// Accepts a new operator on both paths, once the current session has gone
    /// quiet for `takeover_silence`. Sequence numbers keep rising across the
    /// change, so an old command cannot be replayed to the new session.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError::Timeout`] if no operator is accepted in time.
    pub fn reaccept(
        &mut self,
        identity: &DeviceIdentity,
        trust: &TrustStore,
        timeout: Duration,
        takeover_silence: Duration,
    ) -> Result<(), TransportError> {
        self.command
            .accept(identity, trust, timeout, takeover_silence)?;
        self.feed.accept(identity, trust, timeout, takeover_silence)
    }

    /// Returns the next command from the operator, if one has arrived. A
    /// command whose sequence number does not rise is dropped.
    ///
    /// # Errors
    ///
    /// Returns a socket error, or [`TransportError::NotEstablished`] before accept.
    pub fn recv_command(&mut self) -> Result<Option<ControlCommand>, TransportError> {
        let Some(n) = self.command.recv_open()? else {
            return Ok(None);
        };
        let Ok(arch) = access_root::<ControlCommand>(&self.command.plain[..n]) else {
            return Ok(None);
        };
        let cmd = UdpMux::command_from_archived(arch);
        if cmd.magic != MAGIC_COMMAND || self.command.last_seq.is_some_and(|s| cmd.seq <= s) {
            return Ok(None);
        }
        self.command.last_seq = Some(cmd.seq);
        Ok(Some(cmd))
    }

    /// Sends one telemetry packet to the operator.
    ///
    /// # Errors
    ///
    /// Returns [`TransportError::NotEstablished`] before accept, or a socket or
    /// crypto error.
    pub fn send_feed(&mut self, values: [f32; 8], now_ns: u64) -> Result<(), TransportError> {
        let pkt = TelemetryPacket {
            magic: TelemetryPacket::MAGIC,
            kind: TelemetryKind::Generic as u16,
            reserved: 0,
            seq: self.next_seq,
            timestamp_ns: now_ns,
            values,
        };
        self.next_seq += 1;
        let n = serialize_into(&pkt, &mut self.scratch).map_err(|_| TransportError::Crypto)?;
        self.feed.send_sealed(&self.scratch[..n], now_ns)
    }
}

/// A serialization buffer of the largest payload size, so no send allocates.
fn sized_scratch() -> AlignedBuf {
    let mut buf = AlignedBuf::new();
    buf.resize(MAX_PAYLOAD, 0);
    buf
}
