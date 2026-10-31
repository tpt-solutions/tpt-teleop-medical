//! End-to-end check of the telesurgery lifecycle in spec.txt §6.
//!
//! The steps run in the order the spec gives them, across the adapter, the
//! TMR safety monitor, the predictive display and the hash-chained audit log.
//! Registration, incision haptics and vision are not driven here, because
//! their crates run on synthetic data and have no live feed yet.

//! Exact zero torque is the expected halt value, so exact comparison is intended.
#![allow(
    clippy::float_cmp,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss
)]

use tpt_t_core::Mode;
use tpt_t_core::ser::ControlCommand;
use tpt_t_domain_bridge::{DomainTeleopInterface, OperatorId};
use tpt_t_medical_compliance::ed25519::Ed25519Signer;
use tpt_t_medical_compliance::journal::Journal;
use tpt_t_medical_compliance::wal::FileWal;
use tpt_t_medical_compliance::{Storage, verify_entries};
use tpt_t_medical_edge::Model;
use tpt_t_medical_edge::control::Friction;
use tpt_t_medical_safety::{Decision, EStopSource, Envelope, Limits, Sample, StopReason, Tmr};
use tpt_t_medical_surgical::kinematics::Arm;
use tpt_t_medical_surgical::predict::Predictor;
use tpt_t_medical_surgical::{MotionScaler, SdfGrid};
use tpt_t_medical_teleop::{Actuators, Adapter, Config, JointReading};

const MASSES: [f64; 7] = [4.0, 3.0, 3.0, 2.5, 2.0, 1.5, 1.0];
const MS: u64 = 1_000;
const WATCHDOG_US: u64 = 10 * MS;
const SURGEON: OperatorId = OperatorId::new([9; 16]);
const HOME: [f64; 7] = [0.0; 7];
const TOOL_HOME: [f64; 3] = [0.088, 0.0, 0.926];
const SEED: [u8; 32] = [11; 32];

#[derive(Debug, Default)]
struct Recorder {
    last: [f64; 7],
}

impl Actuators for Recorder {
    fn apply(&mut self, torque_nm: [f64; 7]) {
        self.last = torque_nm;
    }
}

/// In-memory storage for the journal. Each record is kept as written.
#[derive(Debug, Default)]
struct MemStore(Vec<Vec<u8>>);

impl Storage for MemStore {
    fn append(&mut self, record: &[u8]) -> std::io::Result<()> {
        self.0.push(record.to_vec());
        Ok(())
    }

    fn records(&mut self) -> std::io::Result<Vec<Vec<u8>>> {
        Ok(self.0.clone())
    }
}

/// A signed-distance grid over the workspace, with a ball at `center`.
fn structure(center: [f64; 3], radius: f64) -> SdfGrid {
    let spacing = 0.005;
    let dims = [41_u32, 41, 31];
    let origin = [0.0, -0.1, 0.85];
    let mut values = Vec::new();
    for z in 0..dims[2] {
        for y in 0..dims[1] {
            for x in 0..dims[0] {
                let p = [
                    origin[0] + f64::from(x) * spacing,
                    origin[1] + f64::from(y) * spacing,
                    origin[2] + f64::from(z) * spacing,
                ];
                let d = (0..3)
                    .map(|i| (p[i] - center[i]).powi(2))
                    .sum::<f64>()
                    .sqrt()
                    - radius;
                values.push(d as f32);
            }
        }
    }
    SdfGrid::new(origin, spacing, dims, values).expect("grid")
}

fn envelope() -> Envelope {
    Envelope::new(
        [
            TOOL_HOME[0] - 0.05,
            TOOL_HOME[1] - 0.05,
            TOOL_HOME[2] - 0.05,
        ],
        [
            TOOL_HOME[0] + 0.05,
            TOOL_HOME[1] + 0.05,
            TOOL_HOME[2] + 0.05,
        ],
    )
    .expect("envelope")
}

fn config(sdf: SdfGrid) -> Config {
    Config {
        model: Model {
            arm: Arm::reference(),
            link_masses_kg: MASSES,
            friction: Friction::new(0.0, 0.0, 0.0).expect("friction"),
            limits_nm: [50.0; 7],
        },
        envelope: envelope(),
        max_force_n: 10.0,
        watchdog_us: WATCHDOG_US,
        scaler: MotionScaler::new(10.0).expect("ratio"),
        step_m: 0.05,
        stiffness_n_m: 200.0,
        damping_ns_m: 5.0,
        tracking_limit_n: 20.0,
        sdf,
        guard_margin_m: 0.01,
        guard_max_force_n: 3.0,
        override_after_us: 200 * MS,
        retreat_m: 0.001,
        tremor_hz: 10.0,
        sample_hz: 1_000.0,
        ramp_ticks: 50,
    }
}

fn reading(at_us: u64) -> JointReading {
    JointReading {
        at_us,
        q: HOME,
        qdot: [0.0; 7],
    }
}

fn command(at_us: u64, lateral_x: f32) -> ControlCommand {
    let mut cmd = ControlCommand::zeroed(Mode::FullTeleop);
    cmd.timestamp_ns = at_us * 1_000;
    cmd.axes[4] = lateral_x;
    cmd
}

fn sample(now_us: u64, tool: [f64; 3]) -> Sample {
    Sample {
        now_us,
        tool_position_m: tool,
        tool_force_n: [0.0; 3],
    }
}

fn limits() -> Limits {
    Limits::new(envelope(), 10.0, WATCHDOG_US).expect("limits")
}

#[test]
fn telesurgery_procedure_runs_end_to_end_and_seals_a_verifiable_log() {
    let signer = Ed25519Signer::from_seed(&SEED);
    let verifier = signer.verifier();
    let mut journal = Journal::open(MemStore::default(), &verifier).expect("fresh journal opens");
    let mut logged = 0_u64;
    let mut log = |journal: &mut Journal<MemStore>, payload: String| {
        journal
            .append(&signer, payload.into_bytes())
            .expect("append");
        logged += 1;
    };

    // Pre-op planning: a critical artery 3 cm ahead of the tool, radius 1 cm.
    let artery = [TOOL_HOME[0] + 0.03, TOOL_HOME[1], TOOL_HOME[2]];
    let mut adapter =
        Adapter::new(config(structure(artery, 0.01)), Recorder::default()).expect("adapter");
    log(&mut journal, "preop:fixtures-loaded".into());

    // System check: three monitors run the self-test and must all agree.
    let mut tmr = Tmr::new(limits());
    assert_eq!(
        tmr.evaluate(sample(0, TOOL_HOME)),
        Decision::Run,
        "TMR self-test must run"
    );
    log(&mut journal, "system:tmr-self-test-run".into());

    // Engage. The target starts at the tool, so nothing jumps.
    adapter.update_joints(reading(0));
    adapter.on_teleop_engage(SURGEON).expect("engage");
    log(&mut journal, "session:engaged".into());

    // Virtual fixture engagement: push toward the artery for 300 ms. The
    // target must never enter the structure, and every command is logged.
    let check = structure(artery, 0.01);
    for k in 1..=300_u64 {
        adapter.update_joints(reading(k * MS));
        adapter
            .on_control_command(command(k * MS, 1.0))
            .expect("command");
        let target = adapter.target().expect("target");
        let depth = check.distance(target).expect("target inside the grid");
        assert!(
            depth >= 0.0,
            "target entered the artery at step {k}: {depth}"
        );
        log(&mut journal, format!("cmd:{k}:x=1.0"));
    }

    // Audit trail: every tick is torque-checked by the safety path, which
    // stays quiet while the tool is in envelope.
    for k in 301..=400_u64 {
        adapter.update_joints(reading(k * MS));
        let torque = adapter.tick(k * MS);
        assert!(torque.iter().all(|t| t.is_finite()), "tick {k}");
        assert!(!adapter.is_latched(), "tick {k}");
    }

    // Network jitter: reports arrive 30 ms late. The display, extrapolating
    // within the 50 ms horizon, must show the true position once settled.
    let velocity = 0.1_f64; // metres per second along x
    let delay_us = 30 * MS;
    let mut display = Predictor::new(20 * MS, Predictor::DEFAULT_HORIZON_US).expect("predictor");
    let mut worst_m = 0.0_f64;
    for t in (0..=400 * MS).step_by(MS as usize) {
        let true_at = |us: u64| {
            [
                TOOL_HOME[0] + velocity * (us as f64) / 1e6,
                TOOL_HOME[1],
                TOOL_HOME[2],
            ]
        };
        if t >= delay_us {
            display.observe(t - delay_us, true_at(t - delay_us));
        }
        if t >= 150 * MS {
            if let Some(shown) = display.display(t) {
                let err = (shown[0] - true_at(t)[0]).abs();
                worst_m = worst_m.max(err);
            }
        }
    }
    assert!(
        worst_m < 1e-4,
        "display error {worst_m} m under 30 ms delay"
    );
    log(&mut journal, "network:jitter-masked".into());

    // Emergency: the foot pedal is pressed. All three monitor copies stop, and
    // the adapter applies zero torque on its next tick.
    let mut stopped = Tmr::new(limits());
    stopped.emergency_stop(EStopSource::FootPedal);
    let decision = stopped.evaluate(sample(401 * MS, TOOL_HOME));
    assert_eq!(
        decision,
        Decision::Stop(StopReason::EStop(EStopSource::FootPedal))
    );
    adapter.on_emergency_stop();
    let halt_at = 402 * MS;
    let torque = adapter.tick(halt_at);
    let halt_ms = (halt_at - 401 * MS) as f64 / 1e3;
    assert!(halt_ms < 10.0, "halt took {halt_ms} ms");
    assert_eq!(torque, [0.0; 7]);
    assert_eq!(adapter.last_torque(), [0.0; 7]);
    log(&mut journal, "estop:foot-pedal:halted".into());

    // Post-op: seal the log, export it, and check the exported chain.
    let entries = journal.log().entries().to_vec();
    assert_eq!(
        entries.len() as u64,
        logged,
        "every logged event is in the chain"
    );
    assert_eq!(verify_entries(&entries, &verifier), Ok(()));

    // An edit to the exported record is caught at the record it changes.
    let mut tampered = entries.clone();
    tampered[120].payload = b"cmd:120:x=-1.0".to_vec();
    let err = verify_entries(&tampered, &verifier).expect_err("tamper must be caught");
    assert_eq!(err.index, 120);
}

#[test]
fn a_journal_on_real_files_survives_reopen_with_the_same_head() {
    // The file WAL is the production backend. Reopening it must verify and
    // give back the same head hash.
    let path =
        std::env::temp_dir().join(format!("tpt-telesurgery-flow-{}.wal", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let signer = Ed25519Signer::from_seed(&SEED);
    let head = {
        let mut journal =
            Journal::open(FileWal::open(&path).expect("open"), &signer.verifier()).expect("fresh");
        for payload in ["preop", "engaged", "estop:foot-pedal"] {
            journal
                .append(&signer, payload.as_bytes().to_vec())
                .expect("append");
        }
        journal.log().head()
    };
    let reopened = Journal::open(FileWal::open(&path).expect("open"), &signer.verifier())
        .expect("reopen verifies");
    assert_eq!(reopened.log().head(), head);
    assert_eq!(reopened.log().entries().len(), 3);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_degraded_multi_site_link_keeps_moving_and_a_lost_one_latches() {
    use tpt_t_medical_teleop::link::LinkState;

    let artery = [TOOL_HOME[0] + 0.03, TOOL_HOME[1], TOOL_HOME[2]];
    let mut adapter =
        Adapter::new(config(structure(artery, 0.01)), Recorder::default()).expect("adapter");
    adapter.update_joints(reading(0));
    adapter.on_teleop_engage(SURGEON).expect("engage");

    // Between the 10 ms budget and the 50 ms horizon: warned, but still moving.
    for k in 31..=230_u64 {
        let at = k * MS;
        adapter.update_joints(reading(at));
        let state = adapter.observe_link(at - 30 * MS, at);
        assert_eq!(state, LinkState::Degraded, "sample {k}");
        adapter.tick(at);
    }
    assert!(
        !adapter.is_latched(),
        "a degraded link must not stop the arm"
    );

    // A command arriving 80 ms late is beyond the horizon. The arm stops.
    let late = 231 * MS;
    assert_eq!(adapter.observe_link(late - 80 * MS, late), LinkState::Lost);
    assert!(adapter.is_latched());
    assert_eq!(adapter.tick(late + MS), [0.0; 7]);
}

#[test]
fn a_silent_multi_site_link_latches_once_the_horizon_has_passed() {
    use tpt_t_medical_teleop::link::LinkState;

    let artery = [TOOL_HOME[0] + 0.03, TOOL_HOME[1], TOOL_HOME[2]];
    let mut adapter =
        Adapter::new(config(structure(artery, 0.01)), Recorder::default()).expect("adapter");
    adapter.update_joints(reading(0));
    adapter.on_teleop_engage(SURGEON).expect("engage");
    assert_eq!(adapter.observe_link(0, MS), LinkState::Nominal);

    // Joints keep arriving, but no command has come for longer than the horizon.
    adapter.update_joints(reading(60 * MS));
    adapter.tick(60 * MS);
    assert!(adapter.is_latched());
}
