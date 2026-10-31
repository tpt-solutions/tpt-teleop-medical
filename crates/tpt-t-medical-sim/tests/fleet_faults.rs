//! Simulator scenarios for fleet dispatch (`docs/design/ai-session.md` §5).
//! Each unit has its own session and journal. These scenarios check that one
//! unit's fault stays with that unit, and that the dispatcher cannot clear it.

use std::cell::RefCell;
use std::io;
use std::rc::Rc;

use tpt_t_medical_ai_session::audit::SessionAudit;
use tpt_t_medical_ai_session::fleet::Fleet;
use tpt_t_medical_ai_session::{Actor, Command, Config, Session, State, Verdict};
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

/// One unit's session, with the same envelope and timing as the single-unit
/// scenarios. The heartbeat is half a second.
fn unit(id: u32, tool: [f64; 3]) -> SessionAudit<Store, Ed25519Signer> {
    let min = tool.map(|v| v - 0.05);
    let max = tool.map(|v| v + 0.05);
    let envelope = Envelope::new(min, max).expect("envelope");
    let limits = Limits::new(envelope, 5.0, 2 * SEC).expect("limits");
    let config = Config::new(60 * SEC, 2 * SEC, SEC / 10, SEC / 2, 0.005).expect("config");
    let session = Session::new(SURGEON, limits, config).expect("session");
    let signer = Ed25519Signer::from_seed(&[u8::try_from(id % 256).unwrap_or(0); 32]);
    SessionAudit::open(id, session, Store::default(), &signer.verifier(), signer).expect("open")
}

/// Clears the startup latch, as the surgeon would.
fn reset(audit: &mut SessionAudit<Store, Ed25519Signer>, now_us: u64) {
    audit
        .handle(Actor::Surgeon(SURGEON), Command::Reset, now_us)
        .expect("reset");
}

/// Brings a unit to `TeleopActive`: reset, the AI asks, the surgeon engages.
fn engage(audit: &mut SessionAudit<Store, Ed25519Signer>, now_us: u64) {
    reset(audit, now_us);
    audit
        .handle(Actor::Ai, Command::RequestAssistance, now_us)
        .expect("request");
    audit
        .handle(Actor::Surgeon(SURGEON), Command::Engage, now_us)
        .expect("engage");
}

#[test]
fn a_silent_ai_on_one_unit_stops_that_unit_and_no_other() {
    let (mut sim, tool) = arm_at_rest();
    let mut busy = unit(1, tool);
    let mut idle = unit(2, tool);
    engage(&mut busy, 0);
    reset(&mut idle, 0);

    let mut fleet = Fleet::new();
    assert!(fleet.add(busy));
    assert!(fleet.add(idle));

    let mut now = 0;
    let mut changes = Vec::new();
    while now < 2 * SEC && changes.is_empty() {
        now += STEP_US;
        sim.step(Some([0.0; JOINTS]));
        changes = fleet.tick_all(now).expect("tick");
    }

    assert_eq!(
        changes,
        vec![(
            1,
            Verdict::Faulted(StopReason::EStop(EStopSource::Automatic))
        )],
        "only the silent unit may change"
    );
    let summary = fleet.summary();
    assert_eq!(summary.stopped, vec![1]);
    assert!(summary.active.is_empty());
    assert_eq!(fleet.state(2), Some(State::Autonomous));
}

#[test]
fn the_dispatcher_cannot_clear_a_stop_on_any_unit() {
    let (mut sim, tool) = arm_at_rest();
    let mut busy = unit(1, tool);
    engage(&mut busy, 0);

    let mut fleet = Fleet::new();
    assert!(fleet.add(busy));

    let mut now = 0;
    while fleet.summary().stopped.is_empty() && now < 2 * SEC {
        now += STEP_US;
        sim.step(Some([0.0; JOINTS]));
        fleet.tick_all(now).expect("tick");
    }
    assert_eq!(fleet.summary().stopped, vec![1]);

    let asked = fleet.request_assistance(1, Actor::Dispatcher(9), now);
    assert!(asked.is_err(), "a latched unit must refuse a new request");
    assert_eq!(fleet.state(1), Some(State::EmergencyStop));
    assert_eq!(fleet.summary().stopped, vec![1]);
    assert!(fleet.pending().is_empty());
}
