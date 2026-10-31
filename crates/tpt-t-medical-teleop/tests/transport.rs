//! The network transport over loopback UDP, with real handshakes and sealed
//! datagrams.

// The telemetry values are copied through unchanged, so exact comparison is the
// point of the check.
#![allow(clippy::float_cmp)]

use std::net::SocketAddr;
use std::thread;
use std::time::{Duration, Instant};

use tpt_t_core::Mode;
use tpt_t_core::ser::ControlCommand;
use tpt_t_link::mux::{MAX_DATAGRAM, UdpMux, secure_inner};
use tpt_t_medical_teleop::transport::{Arm, Feed, Operator, TransportError};
use tpt_t_sec::{DeviceIdentity, Role, TrustStore};

const TIMEOUT: Duration = Duration::from_secs(2);
const SHORT: Duration = Duration::from_millis(300);
const SILENCE: Duration = Duration::from_secs(1);

fn identity(unit: u64, role: Role) -> DeviceIdentity {
    DeviceIdentity::generate(unit, role).expect("identity")
}

/// A trust store that enrolls each identity with the given role.
fn enrolled(entries: &[(&DeviceIdentity, Role)]) -> TrustStore {
    let mut trust = TrustStore::new();
    for (id, role) in entries {
        trust.enroll(id.unit_id(), id.public_key(), *role);
    }
    trust
}

fn command(throttle: f32) -> ControlCommand {
    let mut cmd = ControlCommand::zeroed(Mode::FullTeleop);
    cmd.axes[3] = throttle;
    cmd
}

/// Polls `f` until it yields a value, or fails after the timeout.
fn wait<T>(mut f: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Some(value) = f() {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for a datagram"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

/// The arm's sockets and the addresses the operator needs.
struct Ports {
    arm_command: UdpMux,
    arm_feed: UdpMux,
    op_command: UdpMux,
    op_feed: UdpMux,
    arm_command_addr: SocketAddr,
    arm_feed_addr: SocketAddr,
}

fn ports() -> Ports {
    let arm_command = UdpMux::bind_loopback().expect("bind");
    let arm_feed = UdpMux::bind_loopback().expect("bind");
    let arm_command_addr = arm_command.local_addr().expect("addr");
    let arm_feed_addr = arm_feed.local_addr().expect("addr");
    Ports {
        arm_command,
        arm_feed,
        op_command: UdpMux::bind_loopback().expect("bind"),
        op_feed: UdpMux::bind_loopback().expect("bind"),
        arm_command_addr,
        arm_feed_addr,
    }
}

/// The arm's listening addresses, for any later connection to it.
#[derive(Clone, Copy)]
struct ArmAddrs {
    command: SocketAddr,
    feed: SocketAddr,
}

/// Connects an operator to an arm that accepts on another thread. The arm's
/// accept is given a silence window of `silence`. The arm's addresses come back
/// too, so a later operator can connect to the same sockets.
fn pair(
    op: &DeviceIdentity,
    arm: &DeviceIdentity,
    op_trust: &TrustStore,
    arm_trust: &TrustStore,
    silence: Duration,
) -> (Operator, Arm, ArmAddrs) {
    let p = ports();
    let addrs = ArmAddrs {
        command: p.arm_command_addr,
        feed: p.arm_feed_addr,
    };
    let (operator, arm_end) = thread::scope(|s| {
        let accepting = s.spawn(move || {
            Arm::accept(
                arm,
                arm_trust,
                p.arm_command,
                p.arm_feed,
                0,
                TIMEOUT,
                silence,
            )
            .expect("arm accepts")
        });
        let operator = Operator::connect(
            op,
            op_trust,
            arm.unit_id(),
            addrs.command,
            addrs.feed,
            p.op_command,
            p.op_feed,
            0,
            TIMEOUT,
        )
        .expect("operator connects");
        (operator, accepting.join().expect("arm thread"))
    });
    (operator, arm_end, addrs)
}

#[test]
fn commands_and_telemetry_cross_the_sealed_paths() {
    let op = identity(1, Role::Operator);
    let arm = identity(2, Role::Admin);
    let op_trust = enrolled(&[(&arm, Role::Admin)]);
    let arm_trust = enrolled(&[(&op, Role::Operator)]);
    let (mut operator, mut arm_end, _) = pair(&op, &arm, &op_trust, &arm_trust, SILENCE);

    operator.send_command(command(0.5), 1_000).expect("send");
    let received = wait(|| arm_end.recv_command().expect("recv"));
    assert!((received.axes[3] - 0.5).abs() < f32::EPSILON);
    assert_eq!(received.timestamp_ns, 1_000);

    let values = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 0.0];
    arm_end.send_feed(values, 2_000).expect("feed");
    let feed: Feed = wait(|| operator.recv_feed().expect("recv"));
    assert_eq!(feed.values, values);
    assert_eq!(feed.timestamp_ns, 2_000);
}

#[test]
fn an_unenrolled_operator_gets_no_session() {
    let op = identity(1, Role::Operator);
    let arm = identity(2, Role::Admin);
    let op_trust = enrolled(&[(&arm, Role::Admin)]);
    let arm_trust = TrustStore::new();
    let p = ports();
    thread::scope(|s| {
        let accepting = s.spawn(move || {
            Arm::accept(
                &arm,
                &arm_trust,
                p.arm_command,
                p.arm_feed,
                0,
                SHORT,
                SILENCE,
            )
        });
        let result = Operator::connect(
            &op,
            &op_trust,
            2,
            p.arm_command_addr,
            p.arm_feed_addr,
            p.op_command,
            p.op_feed,
            0,
            SHORT,
        );
        assert!(matches!(result, Err(TransportError::Timeout)));
        assert!(matches!(
            accepting.join().expect("arm thread"),
            Err(TransportError::Timeout)
        ));
    });
}

#[test]
fn a_role_that_may_not_command_is_refused() {
    let op = identity(1, Role::Observer);
    let arm = identity(2, Role::Admin);
    let op_trust = enrolled(&[(&arm, Role::Admin)]);
    let arm_trust = enrolled(&[(&op, Role::Observer)]);
    let p = ports();
    thread::scope(|s| {
        let accepting = s.spawn(move || {
            Arm::accept(
                &arm,
                &arm_trust,
                p.arm_command,
                p.arm_feed,
                0,
                SHORT,
                SILENCE,
            )
        });
        let result = Operator::connect(
            &op,
            &op_trust,
            2,
            p.arm_command_addr,
            p.arm_feed_addr,
            p.op_command,
            p.op_feed,
            0,
            SHORT,
        );
        assert!(matches!(result, Err(TransportError::Timeout)));
        assert!(accepting.join().expect("arm thread").is_err());
    });
}

#[test]
fn an_operator_refuses_an_arm_it_did_not_expect() {
    let op = identity(1, Role::Operator);
    let arm = identity(2, Role::Admin);
    let op_trust = enrolled(&[(&arm, Role::Admin)]);
    let arm_trust = enrolled(&[(&op, Role::Operator)]);
    let p = ports();
    thread::scope(|s| {
        let accepting = s.spawn(move || {
            Arm::accept(
                &arm,
                &arm_trust,
                p.arm_command,
                p.arm_feed,
                0,
                SHORT,
                SILENCE,
            )
        });
        // The operator expects unit 9, but unit 2 answers.
        let result = Operator::connect(
            &op,
            &op_trust,
            9,
            p.arm_command_addr,
            p.arm_feed_addr,
            p.op_command,
            p.op_feed,
            0,
            SHORT,
        );
        assert!(matches!(result, Err(TransportError::Untrusted)));
        let _ = accepting.join();
    });
}

#[test]
fn forged_datagrams_are_dropped_and_real_commands_still_arrive() {
    let op = identity(1, Role::Operator);
    let arm = identity(2, Role::Admin);
    let op_trust = enrolled(&[(&arm, Role::Admin)]);
    let arm_trust = enrolled(&[(&op, Role::Operator)]);
    let p = ports();
    let arm_command_addr = p.arm_command_addr;
    let (mut operator, mut arm_end) = {
        let op_ref = &op;
        let arm_ref = &arm;
        thread::scope(|s| {
            let accepting = s.spawn(move || {
                Arm::accept(
                    arm_ref,
                    &arm_trust,
                    p.arm_command,
                    p.arm_feed,
                    0,
                    TIMEOUT,
                    SILENCE,
                )
                .expect("arm accepts")
            });
            let operator = Operator::connect(
                op_ref,
                &op_trust,
                2,
                arm_command_addr,
                p.arm_feed_addr,
                p.op_command,
                p.op_feed,
                0,
                TIMEOUT,
            )
            .expect("operator connects");
            (operator, accepting.join().expect("arm thread"))
        })
    };

    // An attacker sends a sealed-looking datagram with no key behind it.
    let mut attacker = UdpMux::bind_loopback().expect("bind");
    let mut buf = [0u8; MAX_DATAGRAM];
    let n = attacker
        .write_secure_frame(&[7u8; 64], secure_inner::CONTROL, &mut buf)
        .expect("frame");
    attacker
        .send_framed(arm_command_addr, &buf[..n], 0)
        .expect("send");
    thread::sleep(Duration::from_millis(50));
    assert!(arm_end.recv_command().expect("recv").is_none());

    operator.send_command(command(0.25), 3_000).expect("send");
    let received = wait(|| arm_end.recv_command().expect("recv"));
    assert!((received.axes[3] - 0.25).abs() < f32::EPSILON);
}

#[test]
fn a_live_session_cannot_be_taken_over() {
    let op = identity(1, Role::Operator);
    let other = identity(3, Role::Operator);
    let arm = identity(2, Role::Admin);
    let op_trust = enrolled(&[(&arm, Role::Admin)]);
    let arm_trust = enrolled(&[(&op, Role::Operator), (&other, Role::Operator)]);
    let (mut operator, mut arm_end, addrs) = pair(&op, &arm, &op_trust, &arm_trust, SILENCE);
    operator.send_command(command(0.1), 1_000).expect("send");
    wait(|| arm_end.recv_command().expect("recv"));

    // The first operator is still sending, so a second one must wait.
    let p = ports();
    let other_trust = enrolled(&[(&arm, Role::Admin)]);
    let (takeover_result, reaccept_result) = thread::scope(|s| {
        let taking_over = s.spawn(move || {
            Operator::connect(
                &other,
                &other_trust,
                2,
                addrs.command,
                addrs.feed,
                p.op_command,
                p.op_feed,
                10_000,
                SHORT,
            )
            .err()
        });
        let reaccept = arm_end.reaccept(&arm, &arm_trust, SHORT, SILENCE);
        (taking_over.join().expect("operator thread"), reaccept)
    });
    assert!(matches!(takeover_result, Some(TransportError::Timeout)));
    assert!(matches!(reaccept_result, Err(TransportError::Timeout)));
}

#[test]
fn a_new_operator_cannot_replay_old_sequence_numbers() {
    let op = identity(1, Role::Operator);
    let other = identity(3, Role::Operator);
    let arm = identity(2, Role::Admin);
    let op_trust = enrolled(&[(&arm, Role::Admin)]);
    let arm_trust = enrolled(&[(&op, Role::Operator), (&other, Role::Operator)]);
    // A short silence window, so the first session can be taken over.
    let short_silence = Duration::from_millis(100);
    let (mut first, mut arm_end, addrs) = pair(&op, &arm, &op_trust, &arm_trust, short_silence);
    first.send_command(command(0.1), 1_000).expect("send");
    first.send_command(command(0.2), 2_000).expect("send");
    wait(|| arm_end.recv_command().expect("recv"));
    wait(|| arm_end.recv_command().expect("recv"));
    thread::sleep(Duration::from_millis(150));

    // The second operator starts its sequence numbers at zero. Its first two
    // commands are old, and only the third is above the arm's last sequence.
    let p = ports();
    let other_trust = enrolled(&[(&arm, Role::Admin)]);
    let accepted = thread::scope(|s| {
        s.spawn(move || {
            let mut second = Operator::connect(
                &other,
                &other_trust,
                2,
                addrs.command,
                addrs.feed,
                p.op_command,
                p.op_feed,
                0,
                TIMEOUT,
            )
            .expect("second operator connects");
            for k in 0..3 {
                second.send_command(command(0.3), 3_000 + k).expect("send");
            }
        });
        arm_end
            .reaccept(&arm, &arm_trust, TIMEOUT, short_silence)
            .expect("arm takes the second operator");
        wait(|| arm_end.recv_command().expect("recv"))
    });
    assert_eq!(accepted.seq, 2);
}
