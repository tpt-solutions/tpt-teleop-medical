//! AI controller as an assist-only operator (docs/design/ai-session.md).
//!
//! The recommendations in the design draft are the rules here:
//!
//! * Assist is the default. In assist mode the AI can only *propose* motion. A
//!   proposal is checked by the safety monitor and then recorded as advice. It
//!   never moves the arm, and the surgeon's command is the only motion source.
//! * Full autonomy is an opt-in mode. The surgeon chooses it before engaging,
//!   and it lasts until the session leaves control. In that mode a checked
//!   proposal is a [`Verdict::Commanded`] motion, still behind the same safety
//!   checks. It is never less checked than assist mode.
//! * Only the operating surgeon can engage, release, override, renew or reset.
//!   The AI and fleet dispatchers can only request assistance.
//! * A session expires unless a surgeon renews it. A missed heartbeat from the
//!   AI is a fault.
//! * A proposal that leaves the envelope or the force limit, or that contradicts
//!   the operator's motion, latches an automatic E-stop.
//! * A session opens latched. A restart therefore never resumes motion without
//!   a surgeon's reset.
//!
//! The state machine is in [`Session`]. [`audit::SessionAudit`] journals every
//! command, and [`fleet::Fleet`] holds several units, each with its own journal.
//! The only route by which the AI reads patient data is [`phi::read_for_ai`].

#![forbid(unsafe_code)]

pub mod audit;
pub mod fleet;
pub mod phi;

use tpt_t_medical_safety::{Decision, EStopSource, Limits, Sample, StopReason, Tmr};

/// How much motion authority the AI has while the surgeon is engaged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// The AI proposes and the surgeon moves the arm. This is the default.
    #[default]
    Assist,
    /// The AI's checked proposals are the commanded motion. Chosen by the
    /// surgeon before engaging, and only then.
    FullAutonomy,
}

/// Where the session is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// The AI is not controlling and not asked to.
    Autonomous,
    /// Assistance was asked for. A surgeon must engage or refuse.
    RequestingTeleop,
    /// The surgeon has engaged. The AI's proposals are advisory.
    TeleopActive,
    /// The surgeon released control. The handover is in progress.
    ReturningToAutonomy,
    /// Stopped and latched. Only a surgeon's reset clears it.
    EmergencyStop,
}

/// Who is acting. The caller authenticates the actor before the call, as the
/// custody ledger does. This crate records the identifier and does not check it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Actor {
    /// The operating surgeon, by identifier.
    Surgeon(u32),
    /// A fleet dispatcher, human or AI, by identifier.
    Dispatcher(u32),
    /// The AI controller itself.
    Ai,
}

/// One AI proposal, with the readings it was made from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Proposal {
    /// The displacement the AI proposes for the tool, in metres.
    pub delta_m: [f64; 3],
    /// The tool tip position now, in metres.
    pub tool_position_m: [f64; 3],
    /// The force on the tool now, in newtons.
    pub tool_force_n: [f64; 3],
    /// The displacement the operator actually produced since the previous
    /// proposal, in metres.
    pub observed_delta_m: [f64; 3],
}

/// What an actor asks the session to do.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    /// Ask a surgeon for help. Allowed for the AI and dispatchers.
    RequestAssistance,
    /// Choose the motion authority for the next engagement. Surgeon only, and
    /// only while a request is waiting. Defaults to [`Mode::Assist`].
    SetMode(Mode),
    /// Take the request up. Surgeon only.
    Engage,
    /// Decline the request. Surgeon only.
    Refuse,
    /// Hand control back, through the handover. Surgeon only.
    Release,
    /// Take control at once, with no handover. Surgeon only.
    Override,
    /// Extend the session lifetime. Surgeon only, before expiry.
    Renew,
    /// An AI proposal. AI only.
    Propose(Proposal),
    /// An emergency stop from any source. Any actor may stop.
    EmergencyStop(EStopSource),
    /// Clear a latched stop. Surgeon only.
    Reset,
}

/// What a command did, when it was accepted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Verdict {
    /// The state changed.
    Transitioned {
        /// The state before.
        from: State,
        /// The state after.
        to: State,
    },
    /// The proposal passed the checks in assist mode. It is advice, and the arm
    /// does not move because of it.
    Advised {
        /// The advised displacement, in metres.
        delta_m: [f64; 3],
    },
    /// The proposal passed the checks in full-autonomy mode. This is the motion
    /// the arm should make. The caller applies it through the adapter.
    Commanded {
        /// The commanded displacement, in metres.
        delta_m: [f64; 3],
    },
    /// The surgeon chose the motion authority for the next engagement.
    ModeSet {
        /// The chosen mode.
        mode: Mode,
    },
    /// The session lifetime was extended to this time.
    Renewed {
        /// The new expiry, in microseconds.
        expires_us: u64,
    },
    /// A safety check failed. The session is now stopped and latched.
    Faulted(StopReason),
}

/// Why a command was refused. A refused command changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// The actor may not issue this command.
    NotPermitted,
    /// The command does not apply in the current state.
    WrongState(State),
    /// The AI proposed again before the minimum interval had passed.
    RateLimited,
    /// The session lifetime has run out. The surgeon must engage again.
    Expired,
}

/// Timing and tolerance settings. Every duration is in microseconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Config {
    lifetime_us: u64,
    handover_us: u64,
    min_interval_us: u64,
    heartbeat_us: u64,
    contradiction_m: f64,
}

impl Config {
    /// Creates settings.
    ///
    /// `lifetime_us` is how long an engaged session lasts without renewal.
    /// `handover_us` is the length of the return to autonomy. `min_interval_us`
    /// is the least gap between two proposals. `heartbeat_us` is the longest the
    /// AI may stay silent while engaged. `contradiction_m` is the operator motion
    /// above which an opposing proposal counts as a contradiction.
    ///
    /// Returns `None` if a duration is zero, or the tolerance is negative or not
    /// finite.
    #[must_use]
    pub fn new(
        lifetime_us: u64,
        handover_us: u64,
        min_interval_us: u64,
        heartbeat_us: u64,
        contradiction_m: f64,
    ) -> Option<Self> {
        let durations_ok = [lifetime_us, handover_us, min_interval_us, heartbeat_us]
            .iter()
            .all(|&d| d > 0);
        (durations_ok && contradiction_m.is_finite() && contradiction_m >= 0.0).then_some(Self {
            lifetime_us,
            handover_us,
            min_interval_us,
            heartbeat_us,
            contradiction_m,
        })
    }
}

/// One AI-assisted session with one unit.
#[derive(Debug, Clone, PartialEq)]
pub struct Session {
    surgeon: u32,
    config: Config,
    state: State,
    mode: Mode,
    monitor: Tmr,
    expires_us: u64,
    handover_done_us: u64,
    last_proposal_us: u64,
}

impl Session {
    /// Creates a session for `surgeon`, latched.
    ///
    /// The session starts in [`State::EmergencyStop`]. The surgeon must reset it
    /// before anything else can happen. This is the restart rule: nothing that
    /// happened before the restart is known, so nothing resumes on its own.
    ///
    /// Returns `None` if the heartbeat is longer than the safety monitor's
    /// watchdog. A silent AI would then trip the watchdog before the heartbeat
    /// check could report it.
    #[must_use]
    pub fn new(surgeon: u32, limits: Limits, config: Config) -> Option<Self> {
        if config.heartbeat_us > limits.watchdog_timeout_us() {
            return None;
        }
        let mut monitor = Tmr::new(limits);
        monitor.emergency_stop(EStopSource::Automatic);
        Some(Self {
            surgeon,
            config,
            state: State::EmergencyStop,
            mode: Mode::Assist,
            monitor,
            expires_us: 0,
            handover_done_us: 0,
            last_proposal_us: 0,
        })
    }

    /// Returns the current state.
    #[must_use]
    pub const fn state(&self) -> State {
        self.state
    }

    /// Returns the surgeon who owns this session.
    #[must_use]
    pub const fn surgeon(&self) -> u32 {
        self.surgeon
    }

    /// Returns the reason the session is latched, if it is.
    #[must_use]
    pub const fn latched(&self) -> Option<StopReason> {
        self.monitor.latched()
    }

    /// Returns the motion authority the surgeon chose for this engagement.
    #[must_use]
    pub const fn mode(&self) -> Mode {
        self.mode
    }

    /// Applies one command at `now_us`.
    ///
    /// On `Ok`, the change has happened. On `Err`, nothing has changed. A safety
    /// fault is an `Ok` with [`Verdict::Faulted`], because the stop it causes is
    /// the intended result.
    ///
    /// # Errors
    ///
    /// Returns the reason the command was refused.
    pub fn step(
        &mut self,
        actor: Actor,
        command: Command,
        now_us: u64,
    ) -> Result<Verdict, Refusal> {
        match command {
            Command::EmergencyStop(source) => {
                let from = self.state;
                self.monitor.emergency_stop(source);
                self.state = State::EmergencyStop;
                Ok(Verdict::Transitioned {
                    from,
                    to: State::EmergencyStop,
                })
            }
            Command::RequestAssistance => {
                if !matches!(actor, Actor::Ai | Actor::Dispatcher(_)) {
                    return Err(Refusal::NotPermitted);
                }
                self.require(State::Autonomous)?;
                // Every engagement starts in assist mode. Autonomy has to be
                // chosen again for each one.
                self.mode = Mode::Assist;
                Ok(self.move_to(State::RequestingTeleop))
            }
            Command::SetMode(mode) => {
                self.require_surgeon(actor)?;
                self.require(State::RequestingTeleop)?;
                self.mode = mode;
                Ok(Verdict::ModeSet { mode })
            }
            Command::Engage => {
                self.require_surgeon(actor)?;
                self.require(State::RequestingTeleop)?;
                self.expires_us = now_us.saturating_add(self.config.lifetime_us);
                self.last_proposal_us = now_us;
                Ok(self.move_to(State::TeleopActive))
            }
            Command::Refuse => {
                self.require_surgeon(actor)?;
                self.require(State::RequestingTeleop)?;
                Ok(self.move_to(State::Autonomous))
            }
            Command::Release => {
                self.require_surgeon(actor)?;
                self.require_active()?;
                self.handover_done_us = now_us.saturating_add(self.config.handover_us);
                Ok(self.move_to(State::ReturningToAutonomy))
            }
            Command::Override => {
                self.require_surgeon(actor)?;
                if matches!(self.state, State::Autonomous | State::EmergencyStop) {
                    return Err(Refusal::WrongState(self.state));
                }
                Ok(self.move_to(State::Autonomous))
            }
            Command::Renew => {
                self.require_surgeon(actor)?;
                self.require_active()?;
                self.require_unexpired(now_us)?;
                self.expires_us = now_us.saturating_add(self.config.lifetime_us);
                Ok(Verdict::Renewed {
                    expires_us: self.expires_us,
                })
            }
            Command::Propose(proposal) => {
                if actor != Actor::Ai {
                    return Err(Refusal::NotPermitted);
                }
                self.require_active()?;
                self.require_unexpired(now_us)?;
                let since = now_us.saturating_sub(self.last_proposal_us);
                if since < self.config.min_interval_us {
                    return Err(Refusal::RateLimited);
                }
                Ok(self.propose(proposal, now_us))
            }
            Command::Reset => {
                self.require_surgeon(actor)?;
                self.require(State::EmergencyStop)?;
                self.monitor.reset();
                Ok(self.move_to(State::Autonomous))
            }
        }
    }

    /// Advances time. Returns a verdict only when the state changed.
    ///
    /// A missed heartbeat while engaged is a fault. Otherwise an engaged session
    /// that has run out of time begins its handover, and a finished handover
    /// returns to autonomy.
    pub fn tick(&mut self, now_us: u64) -> Option<Verdict> {
        match self.state {
            State::TeleopActive => {
                let silent_for = now_us.saturating_sub(self.last_proposal_us);
                if silent_for > self.config.heartbeat_us {
                    return Some(self.fault(StopReason::EStop(EStopSource::Automatic)));
                }
                if now_us >= self.expires_us {
                    self.handover_done_us = now_us.saturating_add(self.config.handover_us);
                    return Some(self.move_to(State::ReturningToAutonomy));
                }
                None
            }
            State::ReturningToAutonomy if now_us >= self.handover_done_us => {
                Some(self.move_to(State::Autonomous))
            }
            _ => None,
        }
    }

    fn propose(&mut self, p: Proposal, now_us: u64) -> Verdict {
        if contradicts(p.delta_m, p.observed_delta_m, self.config.contradiction_m) {
            return self.fault(StopReason::EStop(EStopSource::Automatic));
        }
        // Check the proposal as if the arm were already at the predicted point.
        // The monitor is copied first, so a clean proposal leaves it as it was.
        let predicted = [0, 1, 2].map(|i| p.tool_position_m[i] + p.delta_m[i]);
        let mut trial = self.monitor.clone();
        let sample = Sample {
            now_us,
            tool_position_m: predicted,
            tool_force_n: p.tool_force_n,
        };
        if let Decision::Stop(reason) = trial.evaluate(sample) {
            self.monitor = trial;
            return self.fault(reason);
        }
        self.monitor = trial;
        self.last_proposal_us = now_us;
        match self.mode {
            Mode::Assist => Verdict::Advised { delta_m: p.delta_m },
            Mode::FullAutonomy => Verdict::Commanded { delta_m: p.delta_m },
        }
    }

    fn fault(&mut self, reason: StopReason) -> Verdict {
        if let StopReason::EStop(source) = reason {
            self.monitor.emergency_stop(source);
        }
        self.state = State::EmergencyStop;
        Verdict::Faulted(reason)
    }

    fn move_to(&mut self, to: State) -> Verdict {
        let from = self.state;
        self.state = to;
        Verdict::Transitioned { from, to }
    }

    fn require(&self, expected: State) -> Result<(), Refusal> {
        if self.state == expected {
            Ok(())
        } else {
            Err(Refusal::WrongState(self.state))
        }
    }

    fn require_active(&self) -> Result<(), Refusal> {
        self.require(State::TeleopActive)
    }

    fn require_surgeon(&self, actor: Actor) -> Result<(), Refusal> {
        match actor {
            Actor::Surgeon(id) if id == self.surgeon => Ok(()),
            _ => Err(Refusal::NotPermitted),
        }
    }

    fn require_unexpired(&self, now_us: u64) -> Result<(), Refusal> {
        if now_us >= self.expires_us {
            Err(Refusal::Expired)
        } else {
            Ok(())
        }
    }
}

/// True when the proposal points against the operator's motion and the operator
/// really moved. Both vectors must be finite. A NaN never counts as a
/// contradiction, so it is left to the monitor's own checks.
fn contradicts(proposed: [f64; 3], observed: [f64; 3], tolerance_m: f64) -> bool {
    let dot: f64 = (0..3).map(|i| proposed[i] * observed[i]).sum();
    let observed_len: f64 = observed.iter().map(|v| v * v).sum::<f64>().sqrt();
    dot < 0.0 && observed_len > tolerance_m
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_t_medical_safety::Envelope;

    const SURGEON: u32 = 7;
    const SEC: u64 = 1_000_000;

    fn limits() -> Limits {
        let envelope = Envelope::new([-0.1; 3], [0.1; 3]).expect("box");
        Limits::new(envelope, 5.0, 2 * SEC).expect("limits")
    }

    fn config() -> Config {
        // Lifetime 60 s, handover 2 s, 100 ms between proposals, 500 ms
        // heartbeat, and 5 mm of operator motion counts as a contradiction.
        Config::new(60 * SEC, 2 * SEC, SEC / 10, SEC / 2, 0.005).expect("config")
    }

    /// A session that has been requested and engaged, at time `t`.
    fn engaged(t: u64) -> Session {
        let mut s = Session::new(SURGEON, limits(), config()).expect("session");
        s.step(Actor::Surgeon(SURGEON), Command::Reset, t)
            .expect("reset");
        s.step(Actor::Ai, Command::RequestAssistance, t)
            .expect("request");
        s.step(Actor::Surgeon(SURGEON), Command::Engage, t)
            .expect("engage");
        s
    }

    fn proposal(delta: [f64; 3]) -> Proposal {
        Proposal {
            delta_m: delta,
            tool_position_m: [0.0; 3],
            tool_force_n: [0.0; 3],
            observed_delta_m: [0.0; 3],
        }
    }

    #[test]
    fn a_new_session_starts_latched() {
        let s = Session::new(SURGEON, limits(), config()).expect("session");
        assert_eq!(s.state(), State::EmergencyStop);
        assert_eq!(s.latched(), Some(StopReason::EStop(EStopSource::Automatic)));
    }

    #[test]
    fn only_the_surgeon_resets_a_latched_session() {
        let mut s = Session::new(SURGEON, limits(), config()).expect("session");
        assert_eq!(
            s.step(Actor::Ai, Command::Reset, 0),
            Err(Refusal::NotPermitted)
        );
        assert_eq!(
            s.step(Actor::Surgeon(8), Command::Reset, 0),
            Err(Refusal::NotPermitted)
        );
        s.step(Actor::Surgeon(SURGEON), Command::Reset, 0)
            .expect("reset");
        assert_eq!(s.state(), State::Autonomous);
        assert_eq!(s.latched(), None);
    }

    #[test]
    fn the_ai_and_dispatchers_can_only_request() {
        let mut s = Session::new(SURGEON, limits(), config()).expect("session");
        s.step(Actor::Surgeon(SURGEON), Command::Reset, 0)
            .expect("reset");
        assert_eq!(
            s.step(Actor::Ai, Command::Engage, 0),
            Err(Refusal::NotPermitted)
        );
        assert_eq!(
            s.step(Actor::Dispatcher(1), Command::Engage, 0),
            Err(Refusal::NotPermitted)
        );
        assert_eq!(
            s.step(Actor::Ai, Command::Override, 0),
            Err(Refusal::NotPermitted)
        );
        s.step(Actor::Dispatcher(1), Command::RequestAssistance, 0)
            .expect("dispatchers may request");
        assert_eq!(s.state(), State::RequestingTeleop);
    }

    #[test]
    fn a_different_surgeon_cannot_engage() {
        let mut s = Session::new(SURGEON, limits(), config()).expect("session");
        s.step(Actor::Surgeon(SURGEON), Command::Reset, 0)
            .expect("reset");
        s.step(Actor::Ai, Command::RequestAssistance, 0)
            .expect("request");
        assert_eq!(
            s.step(Actor::Surgeon(8), Command::Engage, 0),
            Err(Refusal::NotPermitted)
        );
        assert_eq!(s.state(), State::RequestingTeleop);
    }

    #[test]
    fn engage_and_refuse_follow_a_request() {
        let mut s = Session::new(SURGEON, limits(), config()).expect("session");
        s.step(Actor::Surgeon(SURGEON), Command::Reset, 0)
            .expect("reset");
        assert_eq!(
            s.step(Actor::Surgeon(SURGEON), Command::Engage, 0),
            Err(Refusal::WrongState(State::Autonomous))
        );
        s.step(Actor::Ai, Command::RequestAssistance, 0)
            .expect("request");
        s.step(Actor::Surgeon(SURGEON), Command::Refuse, 0)
            .expect("refuse");
        assert_eq!(s.state(), State::Autonomous);
    }

    #[test]
    fn proposals_are_advice_only_and_leave_the_arm_alone() {
        let mut s = engaged(0);
        let verdict = s
            .step(
                Actor::Ai,
                Command::Propose(proposal([0.001, 0.0, 0.0])),
                SEC / 5,
            )
            .expect("accepted");
        assert_eq!(
            verdict,
            Verdict::Advised {
                delta_m: [0.001, 0.0, 0.0]
            }
        );
        assert_eq!(s.state(), State::TeleopActive);
    }

    #[test]
    fn only_the_ai_may_propose() {
        let mut s = engaged(0);
        assert_eq!(
            s.step(
                Actor::Surgeon(SURGEON),
                Command::Propose(proposal([0.0; 3])),
                SEC
            ),
            Err(Refusal::NotPermitted)
        );
    }

    #[test]
    fn proposals_are_refused_outside_teleop() {
        let mut s = Session::new(SURGEON, limits(), config()).expect("session");
        s.step(Actor::Surgeon(SURGEON), Command::Reset, 0)
            .expect("reset");
        assert_eq!(
            s.step(Actor::Ai, Command::Propose(proposal([0.0; 3])), 0),
            Err(Refusal::WrongState(State::Autonomous))
        );
    }

    #[test]
    fn proposals_faster_than_the_minimum_interval_are_refused_without_change() {
        let mut s = engaged(0);
        s.step(Actor::Ai, Command::Propose(proposal([0.0; 3])), SEC / 5)
            .expect("first");
        let before = s.clone();
        assert_eq!(
            s.step(Actor::Ai, Command::Propose(proposal([0.0; 3])), SEC / 5 + 1),
            Err(Refusal::RateLimited)
        );
        assert_eq!(s, before);
    }

    #[test]
    fn a_proposal_outside_the_envelope_latches_an_automatic_stop() {
        let mut s = engaged(0);
        let mut p = proposal([0.05, 0.0, 0.0]);
        p.tool_position_m = [0.09, 0.0, 0.0];
        let verdict = s
            .step(Actor::Ai, Command::Propose(p), SEC)
            .expect("applied");
        assert_eq!(verdict, Verdict::Faulted(StopReason::OutsideEnvelope));
        assert_eq!(s.state(), State::EmergencyStop);
        assert_eq!(s.latched(), Some(StopReason::OutsideEnvelope));
    }

    #[test]
    fn a_proposal_that_breaks_the_force_limit_latches_a_stop() {
        let mut s = engaged(0);
        let mut p = proposal([0.0; 3]);
        p.tool_force_n = [6.0, 0.0, 0.0];
        let verdict = s
            .step(Actor::Ai, Command::Propose(p), SEC)
            .expect("applied");
        assert_eq!(verdict, Verdict::Faulted(StopReason::ForceExceeded));
    }

    #[test]
    fn a_proposal_that_contradicts_the_operator_is_an_automatic_stop() {
        let mut s = engaged(0);
        let mut p = proposal([0.001, 0.0, 0.0]);
        p.observed_delta_m = [-0.01, 0.0, 0.0];
        let verdict = s
            .step(Actor::Ai, Command::Propose(p), SEC)
            .expect("applied");
        assert_eq!(
            verdict,
            Verdict::Faulted(StopReason::EStop(EStopSource::Automatic))
        );
        assert_eq!(s.state(), State::EmergencyStop);
    }

    #[test]
    fn small_operator_motion_is_not_a_contradiction() {
        let mut s = engaged(0);
        let mut p = proposal([0.001, 0.0, 0.0]);
        p.observed_delta_m = [-0.001, 0.0, 0.0];
        assert!(matches!(
            s.step(Actor::Ai, Command::Propose(p), SEC),
            Ok(Verdict::Advised { .. })
        ));
    }

    #[test]
    fn a_missed_heartbeat_is_a_fault() {
        let mut s = engaged(0);
        assert_eq!(s.tick(SEC / 2), None);
        assert_eq!(
            s.tick(SEC / 2 + 1),
            Some(Verdict::Faulted(StopReason::EStop(EStopSource::Automatic)))
        );
        assert_eq!(s.state(), State::EmergencyStop);
    }

    #[test]
    fn an_engaged_session_expires_into_handover_and_then_autonomy() {
        let mut s = engaged(0);
        // Keep the heartbeat alive until just before the lifetime runs out.
        for i in 1..240 {
            let t = i * SEC / 4;
            s.step(Actor::Ai, Command::Propose(proposal([0.0; 3])), t)
                .expect("heartbeat");
        }
        assert_eq!(
            s.tick(60 * SEC),
            Some(Verdict::Transitioned {
                from: State::TeleopActive,
                to: State::ReturningToAutonomy
            })
        );
        assert_eq!(
            s.step(Actor::Ai, Command::Propose(proposal([0.0; 3])), 60 * SEC),
            Err(Refusal::WrongState(State::ReturningToAutonomy))
        );
        assert_eq!(s.tick(62 * SEC - 1), None);
        assert_eq!(
            s.tick(62 * SEC),
            Some(Verdict::Transitioned {
                from: State::ReturningToAutonomy,
                to: State::Autonomous
            })
        );
    }

    #[test]
    fn renewal_extends_the_lifetime_but_not_after_expiry() {
        let mut s = engaged(0);
        let renewed = s
            .step(Actor::Surgeon(SURGEON), Command::Renew, 30 * SEC)
            .expect("renew");
        assert_eq!(
            renewed,
            Verdict::Renewed {
                expires_us: 90 * SEC
            }
        );
        assert_eq!(
            s.step(Actor::Surgeon(SURGEON), Command::Renew, 90 * SEC),
            Err(Refusal::Expired)
        );
    }

    #[test]
    fn release_hands_back_through_the_handover() {
        let mut s = engaged(0);
        s.step(Actor::Surgeon(SURGEON), Command::Release, SEC)
            .expect("release");
        assert_eq!(s.state(), State::ReturningToAutonomy);
        assert_eq!(
            s.tick(SEC + 2 * SEC),
            Some(Verdict::Transitioned {
                from: State::ReturningToAutonomy,
                to: State::Autonomous
            })
        );
    }

    #[test]
    fn override_takes_effect_at_once() {
        let mut s = engaged(0);
        assert_eq!(
            s.step(Actor::Surgeon(SURGEON), Command::Override, SEC),
            Ok(Verdict::Transitioned {
                from: State::TeleopActive,
                to: State::Autonomous
            })
        );
    }

    #[test]
    fn any_actor_can_stop_and_the_stop_latches_until_reset() {
        let mut s = engaged(0);
        s.step(Actor::Ai, Command::EmergencyStop(EStopSource::Voice), SEC)
            .expect("stop");
        assert_eq!(s.state(), State::EmergencyStop);
        assert_eq!(s.latched(), Some(StopReason::EStop(EStopSource::Voice)));
        assert_eq!(
            s.step(Actor::Surgeon(SURGEON), Command::Engage, SEC),
            Err(Refusal::WrongState(State::EmergencyStop))
        );
    }

    #[test]
    fn invalid_config_and_heartbeat_longer_than_watchdog_are_refused() {
        assert!(Config::new(0, 1, 1, 1, 0.0).is_none());
        assert!(Config::new(1, 1, 1, 1, f64::NAN).is_none());
        let slow = Config::new(60 * SEC, 2 * SEC, SEC / 10, 3 * SEC, 0.005).expect("config");
        assert!(Session::new(SURGEON, limits(), slow).is_none());
    }

    /// A requested session, waiting for the surgeon to choose a mode and engage.
    fn requested(t: u64) -> Session {
        let mut s = Session::new(SURGEON, limits(), config()).expect("session");
        s.step(Actor::Surgeon(SURGEON), Command::Reset, t)
            .expect("reset");
        s.step(Actor::Ai, Command::RequestAssistance, t)
            .expect("request");
        s
    }

    #[test]
    fn a_session_is_assist_only_unless_the_surgeon_chooses_otherwise() {
        assert_eq!(engaged(0).mode(), Mode::Assist);
        assert_eq!(
            Session::new(SURGEON, limits(), config())
                .expect("session")
                .mode(),
            Mode::Assist
        );
    }

    #[test]
    fn only_the_surgeon_chooses_autonomy_and_only_before_engaging() {
        let mut s = requested(0);
        assert_eq!(
            s.step(Actor::Ai, Command::SetMode(Mode::FullAutonomy), 0),
            Err(Refusal::NotPermitted)
        );
        assert_eq!(
            s.step(Actor::Surgeon(8), Command::SetMode(Mode::FullAutonomy), 0),
            Err(Refusal::NotPermitted)
        );
        assert_eq!(
            s.step(
                Actor::Surgeon(SURGEON),
                Command::SetMode(Mode::FullAutonomy),
                0
            ),
            Ok(Verdict::ModeSet {
                mode: Mode::FullAutonomy
            })
        );
        s.step(Actor::Surgeon(SURGEON), Command::Engage, 0)
            .expect("engage");
        assert_eq!(
            s.step(Actor::Surgeon(SURGEON), Command::SetMode(Mode::Assist), SEC),
            Err(Refusal::WrongState(State::TeleopActive))
        );
        assert_eq!(s.mode(), Mode::FullAutonomy);
    }

    #[test]
    fn autonomy_turns_a_checked_proposal_into_commanded_motion() {
        let mut s = requested(0);
        s.step(
            Actor::Surgeon(SURGEON),
            Command::SetMode(Mode::FullAutonomy),
            0,
        )
        .expect("mode");
        s.step(Actor::Surgeon(SURGEON), Command::Engage, 0)
            .expect("engage");
        let verdict = s.step(
            Actor::Ai,
            Command::Propose(proposal([0.001, 0.0, 0.0])),
            SEC,
        );
        assert_eq!(
            verdict,
            Ok(Verdict::Commanded {
                delta_m: [0.001, 0.0, 0.0]
            })
        );
    }

    #[test]
    fn autonomy_is_still_behind_the_envelope_and_the_contradiction_check() {
        let mut s = requested(0);
        s.step(
            Actor::Surgeon(SURGEON),
            Command::SetMode(Mode::FullAutonomy),
            0,
        )
        .expect("mode");
        s.step(Actor::Surgeon(SURGEON), Command::Engage, 0)
            .expect("engage");
        let outside = s.step(Actor::Ai, Command::Propose(proposal([0.2, 0.0, 0.0])), SEC);
        assert_eq!(outside, Ok(Verdict::Faulted(StopReason::OutsideEnvelope)));
        assert_eq!(s.state(), State::EmergencyStop);
    }

    #[test]
    fn autonomy_ends_with_the_engagement_and_the_next_one_starts_in_assist() {
        let mut s = requested(0);
        s.step(
            Actor::Surgeon(SURGEON),
            Command::SetMode(Mode::FullAutonomy),
            0,
        )
        .expect("mode");
        s.step(Actor::Surgeon(SURGEON), Command::Engage, 0)
            .expect("engage");
        s.step(Actor::Surgeon(SURGEON), Command::Override, SEC)
            .expect("override");
        assert_eq!(s.state(), State::Autonomous);
        s.step(Actor::Ai, Command::RequestAssistance, 2 * SEC)
            .expect("request");
        assert_eq!(s.mode(), Mode::Assist);
        s.step(Actor::Surgeon(SURGEON), Command::Engage, 2 * SEC)
            .expect("engage");
        let verdict = s.step(
            Actor::Ai,
            Command::Propose(proposal([0.001, 0.0, 0.0])),
            3 * SEC,
        );
        assert!(
            matches!(verdict, Ok(Verdict::Advised { .. })),
            "{verdict:?}"
        );
    }

    #[test]
    fn a_surgeon_override_ends_autonomy_at_once() {
        let mut s = requested(0);
        s.step(
            Actor::Surgeon(SURGEON),
            Command::SetMode(Mode::FullAutonomy),
            0,
        )
        .expect("mode");
        s.step(Actor::Surgeon(SURGEON), Command::Engage, 0)
            .expect("engage");
        let verdict = s.step(Actor::Surgeon(SURGEON), Command::Override, SEC);
        assert!(matches!(verdict, Ok(Verdict::Transitioned { .. })));
        assert_eq!(s.state(), State::Autonomous);
    }
}
