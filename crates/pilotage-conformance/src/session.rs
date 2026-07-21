//! The scripted-session harness: drives an authority engine and the
//! reference adapter through a fixed script, recording an ordered event log
//! (ADR-0002, ADR-0010, ADR-0012).
//!
//! The harness is itself sans-IO. A [`Script`] is a flat list of
//! [`ScriptStep`]s; each step is one authority command, one control frame
//! routed through authority into the adapter, one adapter step, or one
//! link-loss policy change. [`ScriptedSession::run`] applies them in order
//! and returns the [`SessionEvent`] log plus the final adapter, so callers
//! can compare event logs for determinism and read the adapter trajectory
//! for golden checkpoints.

use core::time::Duration;

use pilotage_adapter_api::{LinkLossPolicy, StepBudget, VehicleAdapter};
use pilotage_adapter_reference::ReferenceAdapter;
use pilotage_authority::{AuthorityCommand, AuthorityEngine, FrameVerdict};
use pilotage_protocol::{ScopeId, ScopedControlFrame, VehicleId};
use pilotage_timing::MonoTimestamp;

use crate::event::{FrameOutcome, SessionEvent};

/// One step in a [`Script`]: exactly one host-observable action.
#[derive(Debug, Clone)]
pub enum ScriptStep {
    /// Submit an authority command at the current session time.
    Command(AuthorityCommand),
    /// Advance the session clock by a duration before the next step; models
    /// wall-clock passing between operator actions so offer TTLs and
    /// staleness can be exercised deterministically.
    AdvanceClock(Duration),
    /// Route a control frame through authority; if accepted, apply it to the
    /// adapter. Records one [`SessionEvent::Frame`].
    Frame(ScopedControlFrame),
    /// Advance the adapter by a tick budget.
    Step(u32),
    /// Set or clear the adapter's link-loss policy for a vehicle.
    LinkLossPolicy {
        /// Vehicle the policy applies to.
        vehicle: VehicleId,
        /// Policy to engage, or `None` to signal link recovery.
        policy: Option<LinkLossPolicy>,
    },
}

/// An ordered fixture script for a whole session.
#[derive(Debug, Clone)]
pub struct Script {
    /// The vehicle whose adapter the session drives.
    pub vehicle: VehicleId,
    /// Deterministic seed for the reference adapter's initial state.
    pub seed: u64,
    /// The ordered steps to apply.
    pub steps: Vec<ScriptStep>,
}

/// A scripted session: an authority engine, the reference adapter, an
/// explicit monotonic clock, and the event log accumulated so far.
///
/// Construct with [`ScriptedSession::new`], drive it with
/// [`ScriptedSession::run`] (whole script) or [`ScriptedSession::apply`]
/// (one step, for snapshot/restore mid-run), and read the results with
/// [`ScriptedSession::events`] and [`ScriptedSession::adapter`].
#[derive(Debug)]
pub struct ScriptedSession {
    engine: AuthorityEngine,
    adapter: ReferenceAdapter,
    now: MonoTimestamp,
    events: Vec<SessionEvent>,
}

impl ScriptedSession {
    /// Creates a session for `vehicle` with the adapter seeded from `seed`.
    #[must_use]
    pub fn new(vehicle: VehicleId, seed: u64) -> Self {
        Self {
            engine: AuthorityEngine::new(),
            adapter: ReferenceAdapter::from_seed(vehicle, seed),
            now: MonoTimestamp::from_nanos(0),
            events: Vec::new(),
        }
    }

    /// Applies every step of `script` in order, returning the accumulated
    /// event log and the final adapter state.
    #[must_use]
    pub fn run(script: &Script) -> (Vec<SessionEvent>, ReferenceAdapter) {
        let mut session = Self::new(script.vehicle, script.seed);
        for step in &script.steps {
            session.apply(step);
        }
        session.into_parts()
    }

    /// Applies a single step, appending any resulting events to the log.
    pub fn apply(&mut self, step: &ScriptStep) {
        match step {
            ScriptStep::Command(command) => self.apply_command(command.clone()),
            ScriptStep::AdvanceClock(delta) => {
                self.now = self.now.saturating_add(*delta);
                self.drain_expired();
            }
            ScriptStep::Frame(frame) => self.apply_frame(frame),
            ScriptStep::Step(ticks) => self.apply_step(*ticks),
            ScriptStep::LinkLossPolicy { vehicle, policy } => {
                self.apply_link_loss_policy(*vehicle, *policy);
            }
        }
    }

    /// Borrows the accumulated event log.
    #[must_use]
    pub fn events(&self) -> &[SessionEvent] {
        &self.events
    }

    /// Borrows the adapter, for reading telemetry checkpoints mid-run.
    #[must_use]
    pub fn adapter(&self) -> &ReferenceAdapter {
        &self.adapter
    }

    /// Consumes the session, returning the event log and the final adapter.
    #[must_use]
    pub fn into_parts(self) -> (Vec<SessionEvent>, ReferenceAdapter) {
        (self.events, self.adapter)
    }

    /// Replaces the adapter with one restored from a snapshot, for exercising
    /// snapshot/restore convergence. The event log and engine are unchanged.
    pub fn replace_adapter(&mut self, adapter: ReferenceAdapter) {
        self.adapter = adapter;
    }

    fn apply_command(&mut self, command: AuthorityCommand) {
        for effect in self.engine.handle(command, self.now) {
            self.events.push(SessionEvent::Authority(effect));
        }
    }

    fn drain_expired(&mut self) {
        for effect in self.engine.expire_due(self.now) {
            self.events.push(SessionEvent::Authority(effect));
        }
    }

    fn apply_frame(&mut self, frame: &ScopedControlFrame) {
        let verdict = self
            .engine
            .verify_frame(frame.vehicle, &frame.scope, frame.generation);
        let (disposition, applied_tick) = if matches!(verdict, FrameVerdict::Accepted) {
            let outcome = self.adapter.apply_control(frame);
            (Some(outcome.disposition), Some(outcome.tick))
        } else {
            (None, None)
        };
        self.events.push(SessionEvent::Frame(FrameOutcome {
            sequence: frame.sequence,
            verdict,
            disposition,
            applied_tick,
        }));
    }

    fn apply_step(&mut self, ticks: u32) {
        let outcome = self.adapter.step(StepBudget { ticks });
        self.events.push(SessionEvent::Stepped {
            advanced: outcome.advanced,
            now: outcome.now,
        });
    }

    fn apply_link_loss_policy(&mut self, vehicle: VehicleId, policy: Option<LinkLossPolicy>) {
        // A refused enactment must be visible in the comparable event log —
        // an adapter that cannot drive its declared policy is a conformance
        // divergence, not a silent no-op. The reference adapter declares only
        // the motion scope, so a link-loss step targets it.
        let scope = ScopeId::new("vehicle.motion");
        let enacted = self.adapter.set_link_loss_policy(vehicle, &scope, policy);
        self.events.push(SessionEvent::LinkLossPolicyEngaged {
            vehicle,
            policy: if enacted.is_ok() {
                policy_label(policy)
            } else {
                "enact-failed"
            },
        });
    }
}

/// Stable label for a link-loss policy, so the event log stays `Eq` without
/// depending on the adapter crate deriving `Eq` on its policy enum.
fn policy_label(policy: Option<LinkLossPolicy>) -> &'static str {
    match policy {
        None => "recovered",
        Some(LinkLossPolicy::Neutralize) => "neutralize",
        Some(LinkLossPolicy::Brake) => "brake",
        Some(LinkLossPolicy::HoldBrief { .. }) => "hold_brief",
        Some(LinkLossPolicy::Pause) => "pause",
        Some(LinkLossPolicy::EngageAutomation) => "engage_automation",
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::{LinkLossPolicy, ScriptStep, ScriptedSession, policy_label};
    use pilotage_authority::AuthorityCommand;
    use pilotage_protocol::{ScopeId, VehicleId};

    #[test]
    fn policy_label_is_stable_per_variant() {
        assert_eq!(policy_label(None), "recovered");
        assert_eq!(policy_label(Some(LinkLossPolicy::Neutralize)), "neutralize");
        assert_eq!(
            policy_label(Some(LinkLossPolicy::HoldBrief { ticks: 3 })),
            "hold_brief"
        );
        assert_eq!(policy_label(Some(LinkLossPolicy::Brake)), "brake");
        assert_eq!(policy_label(Some(LinkLossPolicy::Pause)), "pause");
        assert_eq!(
            policy_label(Some(LinkLossPolicy::EngageAutomation)),
            "engage_automation"
        );
    }

    #[test]
    fn empty_session_produces_no_events() {
        let session = ScriptedSession::new(VehicleId::new(1), 0);
        assert!(session.events().is_empty());
    }

    #[test]
    fn a_register_command_records_one_authority_event() {
        let mut session = ScriptedSession::new(VehicleId::new(1), 0);
        session.apply(&ScriptStep::Command(AuthorityCommand::RegisterScope {
            vehicle: VehicleId::new(1),
            scope: ScopeId::new("vehicle.motion"),
        }));
        assert_eq!(session.events().len(), 1);
        assert!(session.events()[0].as_authority().is_some());
    }
}
