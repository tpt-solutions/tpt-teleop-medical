//! Simulator scenarios for a misbehaving AI controller (`docs/design/ai-session.md`
//! §4). The AI session runs against the simulated arm. Each scenario injects one
//! kind of AI fault and checks that the session latches an automatic stop.

use std::cell::RefCell;
use std::io;
use std::rc::Rc;

use tpt_t_medical_ai_session::audit::SessionAudit;
use tpt_t_medical_ai_session::{Actor, Command, Config, Mode, Proposal, Session, State, Verdict};
use tpt_t_medical_compliance::Storage;
use tpt_t_medical_compliance::ed25519::Ed25519Signer;
use tpt_t_medical_safety::{EStopSource, Envelope, Limits, StopReason};
use tpt_t_medical_sim::{JOINTS, JointParams, Simulator, tool_position};
use tpt_t_medical_surgical::kinematics::Arm;

const SURGEON: u32 = 7;
const SEC: u64 = 1_000_000;
const STEP_US: u64 = 1_000;

#[derive(Clone, Default)]
struct Store(Rc<RefCell<Vec<Vec<u8>>>>);

impl Storage for Store {
    fn append(&mut self, record: &[u8]) -> io::Result<()> {
        self.0.borrow_mut().push(record.to_vec());
        Ok(())
    }

    fn records(&mut self) -> io::Result<Vec<Vec<u8>>> {
        Ok(self.0.borrow().clone())
    }
}

/// A simulated arm at rest, and the tool position it starts at.
fn arm_at_rest() -> (Simulator, [f64; 3]) {
    let params = [JointParams::new(0.5, 0.2, 0.0, 0.0, 0.0).expect("params"); JOINTS];
    let mut sim = Simulator::new(params, [0.0; JOINTS]);
    let readings = sim.step(Some([0.0; JOINTS]));
    let tool = tool_position(&Arm::reference(), &readings).expect("pose");
    (sim, tool)
}

/// An envelope 5 cm either side of where the tool starts.
fn session_around(tool: [f64; 3]) -> Session {
    let min = tool.map(|v| v - 0.05);
    let max = tool.map(|v| v + 0.05);
    let envelope = Envelope::new(min, max).expect("envelope");
    let limits = Limits::new(envelope, 5.0, 2 * SEC).expect("limits");
    let config = Config::new(60 * SEC, 2 * SEC, SEC / 10, SEC / 2, 0.005).expect("config");
    Session::new(SURGEON, limits, config).expect("session")
}

fn audit(session: Session) -> SessionAudit<Store, Ed25519Signer> {
    let signer = Ed25519Signer::from_seed(&[3; 32]);
    SessionAudit::open(1, session, Store::default(), &signer.verifier(), signer).expect("open")
}

/// Brings a latched session to `TeleopActive`, starting at `now_us`.
fn engage(audit: &mut SessionAudit<Store, Ed25519Signer>, now_us: u64) {
    audit
        .handle(Actor::Surgeon(SURGEON), Command::Reset, now_us)
        .expect("reset");
    audit
        .handle(Actor::Ai, Command::RequestAssistance, now_us)
        .expect("request");
    audit
        .handle(Actor::Surgeon(SURGEON), Command::Engage, now_us)
        .expect("engage");
}

fn proposal(sim: &mut Simulator, tool: [f64; 3], delta: [f64; 3], observed: [f64; 3]) -> Proposal {
    let readings = sim.step(Some([0.0; JOINTS]));
    let position = tool_position(&Arm::reference(), &readings).unwrap_or(tool);
    Proposal {
        delta_m: delta,
        tool_position_m: position,
        tool_force_n: [0.0; 3],
        observed_delta_m: observed,
    }
}

#[test]
fn a_well_behaved_ai_is_advice_and_never_stops_the_session() {
    let (mut sim, tool) = arm_at_rest();
    let mut audit = audit(session_around(tool));
    engage(&mut audit, 0);
    let mut now = 0;
    while now < 3 * SEC {
        now += SEC / 5;
        for _ in 0..(SEC / 5 / STEP_US) {
            sim.step(Some([0.0; JOINTS]));
        }
        let p = proposal(&mut sim, tool, [0.0005, 0.0, 0.0], [0.0; 3]);
        let verdict = audit
            .handle(Actor::Ai, Command::Propose(p), now)
            .expect("advised");
        assert!(matches!(verdict, Verdict::Advised { .. }), "{verdict:?}");
    }
    assert_eq!(audit.state(), State::TeleopActive);
}

#[test]
fn an_ai_that_proposes_outside_the_envelope_is_stopped() {
    let (mut sim, tool) = arm_at_rest();
    let mut audit = audit(session_around(tool));
    engage(&mut audit, 0);
    let p = proposal(&mut sim, tool, [0.2, 0.0, 0.0], [0.0; 3]);
    let verdict = audit
        .handle(Actor::Ai, Command::Propose(p), SEC)
        .expect("applied");
    assert_eq!(verdict, Verdict::Faulted(StopReason::OutsideEnvelope));
    assert_eq!(audit.state(), State::EmergencyStop);
}

#[test]
fn an_ai_that_goes_silent_is_stopped_by_the_heartbeat() {
    let (mut sim, tool) = arm_at_rest();
    let mut audit = audit(session_around(tool));
    engage(&mut audit, 0);
    let mut now = 0;
    let mut stopped_at = None;
    while now < 2 * SEC && stopped_at.is_none() {
        now += STEP_US;
        sim.step(Some([0.0; JOINTS]));
        if let Ok(Some(_)) = audit.tick(now) {
            stopped_at = Some(now);
        }
    }
    let at = stopped_at.expect("the session must stop");
    assert!(at <= SEC / 2 + 2 * STEP_US, "stopped late, at {at} us");
    assert_eq!(audit.state(), State::EmergencyStop);
    assert_eq!(
        audit.session().latched(),
        Some(StopReason::EStop(EStopSource::Automatic))
    );
}

#[test]
fn an_ai_that_contradicts_the_operator_is_stopped() {
    let (mut sim, tool) = arm_at_rest();
    let mut audit = audit(session_around(tool));
    engage(&mut audit, 0);
    // The operator moves the tool 1 cm in +x. The AI proposes 1 mm in -x.
    let p = proposal(&mut sim, tool, [-0.001, 0.0, 0.0], [0.01, 0.0, 0.0]);
    let verdict = audit
        .handle(Actor::Ai, Command::Propose(p), SEC)
        .expect("applied");
    assert_eq!(
        verdict,
        Verdict::Faulted(StopReason::EStop(EStopSource::Automatic))
    );
}

#[test]
fn a_latched_fault_stays_latched_until_the_surgeon_resets() {
    let (mut sim, tool) = arm_at_rest();
    let mut audit = audit(session_around(tool));
    engage(&mut audit, 0);
    let p = proposal(&mut sim, tool, [0.2, 0.0, 0.0], [0.0; 3]);
    audit
        .handle(Actor::Ai, Command::Propose(p), SEC)
        .expect("applied");
    let ai_reset = audit.handle(Actor::Ai, Command::Reset, 2 * SEC);
    assert!(ai_reset.is_err(), "the AI must not reset its own stop");
    assert_eq!(audit.state(), State::EmergencyStop);
    audit
        .handle(Actor::Surgeon(SURGEON), Command::Reset, 3 * SEC)
        .expect("surgeon reset");
    assert_eq!(audit.state(), State::Autonomous);
}

/// Brings a latched session to `TeleopActive` in full-autonomy mode.
fn engage_autonomous(audit: &mut SessionAudit<Store, Ed25519Signer>, now_us: u64) {
    audit
        .handle(Actor::Surgeon(SURGEON), Command::Reset, now_us)
        .expect("reset");
    audit
        .handle(Actor::Ai, Command::RequestAssistance, now_us)
        .expect("request");
    audit
        .handle(
            Actor::Surgeon(SURGEON),
            Command::SetMode(Mode::FullAutonomy),
            now_us,
        )
        .expect("mode");
    audit
        .handle(Actor::Surgeon(SURGEON), Command::Engage, now_us)
        .expect("engage");
}

#[test]
fn an_autonomous_ai_moves_the_arm_within_the_envelope() {
    let (mut sim, tool) = arm_at_rest();
    let mut audit = audit(session_around(tool));
    engage_autonomous(&mut audit, 0);
    let mut now = 0;
    let mut commanded = Vec::new();
    while now < 3 * SEC {
        now += SEC / 5;
        for _ in 0..(SEC / 5 / STEP_US) {
            sim.step(Some([0.0; JOINTS]));
        }
        let p = proposal(&mut sim, tool, [0.0005, 0.0, 0.0], [0.0; 3]);
        let verdict = audit
            .handle(Actor::Ai, Command::Propose(p), now)
            .expect("commanded");
        commanded.push(verdict);
    }
    assert!(
        commanded
            .iter()
            .all(|v| matches!(v, Verdict::Commanded { .. })),
        "{commanded:?}"
    );
    assert_eq!(audit.state(), State::TeleopActive);
    assert_eq!(audit.session().mode(), Mode::FullAutonomy);
}

#[test]
fn a_runaway_autonomous_ai_is_stopped_before_it_leaves_the_envelope() {
    let (mut sim, tool) = arm_at_rest();
    let mut audit = audit(session_around(tool));
    engage_autonomous(&mut audit, 0);
    let p = proposal(&mut sim, tool, [0.2, 0.0, 0.0], [0.0; 3]);
    let verdict = audit
        .handle(Actor::Ai, Command::Propose(p), SEC)
        .expect("applied");
    assert_eq!(verdict, Verdict::Faulted(StopReason::OutsideEnvelope));
    assert_eq!(audit.state(), State::EmergencyStop);
}
