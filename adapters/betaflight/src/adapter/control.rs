//! Flight-frame gating and the hand-off to the link: the link-loss latch,
//! structural checks, disarm (never blocked once admitted), the
//! measurement gate, arm with Betaflight's own refusal reasons, then the
//! velocity demand clamped to the envelope.

use std::time::Instant;

use pilotage_adapter_api::{ActionResult, ApplyOutcome, Disposition, RejectReason};
use pilotage_msp::ArmingBlocker;
use pilotage_protocol::{
    ActionKind, ControlAction, ControlIntent, LogicalAxisId, ReferenceFrame, ScopedControlFrame,
    VelocityIntent,
};
use pilotage_timing::SimTick;

use super::{
    BetaflightAdapter, DIRECT_SCOPE, FLIGHT_SCOPE, PITCH_AXIS, ROLL_AXIS, THROTTLE_AXIS, YAW_AXIS,
};
use crate::config::{MAX_HORIZONTAL_MPS, MAX_VERTICAL_MPS, MAX_YAW_RATE_RPS};
use crate::link::Family;
use crate::link::Observation;
use crate::outer_loop::Demand;

fn rejected(tick: SimTick, reason: RejectReason) -> ApplyOutcome {
    ApplyOutcome::new(tick, Disposition::Rejected(reason))
}

impl BetaflightAdapter {
    pub(super) fn apply_flight_frame(&mut self, frame: &ScopedControlFrame) -> ApplyOutcome {
        let tick = self.tick_of(Instant::now());
        if pilotage_adapter_api::link_loss_latch_engaged(&self.link_loss_policy, &frame.scope) {
            return rejected(tick, RejectReason::LinkLossEngaged);
        }
        if let Err(reason) = self.validate(frame) {
            return rejected(tick, reason);
        }
        if frame.actions.iter().any(|a| a.kind() == ActionKind::Disarm) {
            self.commands.send_modify(|command| {
                command.requests.disarm = command.requests.disarm.wrapping_add(1)
            });
            return ApplyOutcome {
                tick,
                disposition: Disposition::Accepted,
                action_results: frame
                    .actions
                    .iter()
                    .map(|action| match action.kind() {
                        ActionKind::Disarm => ActionResult::accepted(*action),
                        _ => ActionResult::rejected(
                            *action,
                            "not executed: the frame's disarm short-circuits it",
                        ),
                    })
                    .collect(),
            };
        }
        let Some(observed) = self.fresh_observation() else {
            return rejected(tick, RejectReason::MeasurementUnavailable);
        };
        let action_results = frame
            .actions
            .iter()
            .map(|action| self.apply_action(*action, &observed))
            .collect();
        let now = Instant::now();
        let constrained = match frame.intent {
            Some(ControlIntent::Velocity(velocity)) => {
                let (demand, constrained) = demand_from_velocity(&velocity);
                self.commands.send_modify(|command| {
                    command.demands.family = Family::Assisted;
                    command.demands.velocity = demand;
                    command.demands.velocity_at = Some(now);
                });
                constrained
            }
            Some(intent) => {
                let Some((demand, constrained)) = super::direct_intent::demand(&intent) else {
                    return rejected(tick, RejectReason::UnknownScope);
                };
                self.commands.send_modify(|command| {
                    command.demands.family = Family::Direct;
                    command.demands.direct = Some(demand);
                    command.demands.direct_at = Some(now);
                });
                constrained
            }
            None => false,
        };
        ApplyOutcome {
            tick,
            disposition: if constrained {
                Disposition::Constrained
            } else {
                Disposition::Accepted
            },
            action_results,
        }
    }

    fn validate(&self, frame: &ScopedControlFrame) -> Result<(), RejectReason> {
        if frame.vehicle != self.vehicle {
            return Err(RejectReason::UnknownVehicle);
        }
        let direct = match frame.scope.as_str() {
            FLIGHT_SCOPE => false,
            DIRECT_SCOPE => true,
            _ => return Err(RejectReason::UnknownScope),
        };
        let known = [ROLL_AXIS, PITCH_AXIS, THROTTLE_AXIS, YAW_AXIS].map(LogicalAxisId::new);
        if frame
            .payload
            .axes
            .iter()
            .any(|(axis, _)| !known.contains(axis))
        {
            return Err(RejectReason::UnknownAxis);
        }
        // Typed-only consumption: the session host translates any legacy
        // payload at its compatibility boundary before delivery.
        if frame.carries_payload() || !frame.carries_typed() {
            return Err(RejectReason::Other(
                "the flight scope consumes typed commands only".to_owned(),
            ));
        }
        // Each scope takes its own families: velocity in BODY_FRD on the
        // assisted scope; body rates, or an attitude in LOCAL_NED, on the
        // direct one.
        let admitted = match (direct, frame.intent) {
            (_, None) => true,
            (false, Some(ControlIntent::Velocity(velocity))) => {
                velocity.frame == ReferenceFrame::BodyFrd
            }
            (true, Some(ControlIntent::BodyRate(_))) => true,
            (true, Some(ControlIntent::AttitudeThrust(attitude))) => {
                attitude.frame == ReferenceFrame::LocalNed
            }
            _ => false,
        };
        if !admitted {
            return Err(RejectReason::Other(if direct {
                "the direct scope takes body rates, or an attitude in LOCAL_NED, with thrust"
                    .to_owned()
            } else {
                "the flight scope takes BODY_FRD velocity only".to_owned()
            }));
        }
        Ok(())
    }

    fn apply_action(&mut self, action: ControlAction, observed: &Observation) -> ActionResult {
        if action.kind() != ActionKind::Arm {
            return ActionResult::rejected(action, "not supported on the flight scope");
        }
        let issue = self.reports.borrow().issue;
        if let Some(issue) = issue {
            self.count_refusal();
            return ActionResult::rejected(action, issue.to_string());
        }
        if observed.snapshot.armed {
            return ActionResult::accepted(action);
        }
        // A stopped PID loop also stops the arming check that would report
        // it, so the arming flags freeze at their boot values: name the
        // real reason instead of a stale one.
        if observed.snapshot.cycle_time_us == 0 {
            self.count_refusal();
            return ActionResult::rejected(
                action,
                "Betaflight's control loop is not running (PID cycle time 0); restart it",
            );
        }
        let blockers = arm_blockers(observed);
        if !blockers.is_empty() {
            self.count_refusal();
            let names: Vec<&str> = blockers.iter().map(|b| b.cli_name()).collect();
            return ActionResult::rejected(
                action,
                format!("Betaflight refuses to arm: {}", names.join(", ")),
            );
        }
        self.commands
            .send_modify(|command| command.requests.arm = command.requests.arm.wrapping_add(1));
        ActionResult::accepted(action)
    }

    fn count_refusal(&mut self) {
        self.commands.send_modify(|command| {
            command.requests.refused = command.requests.refused.wrapping_add(1);
        });
    }
}

/// What blocks arming, without the reason that only echoes the others:
/// Betaflight sets ARM_SWITCH whenever the switch is on while anything
/// else blocks.
pub(super) fn arm_blockers(observed: &Observation) -> Vec<ArmingBlocker> {
    observed
        .snapshot
        .arming_disable
        .blockers()
        .into_iter()
        .filter(|blocker| *blocker != ArmingBlocker::ArmSwitch)
        .collect()
}

/// The typed velocity as a demand inside the envelope, and whether it was
/// clamped. Non-finite components read as zero.
pub(super) fn demand_from_velocity(velocity: &VelocityIntent) -> (Demand, bool) {
    let clamp = |value: f32, limit: f32| {
        let value = if value.is_finite() { value } else { 0.0 };
        let clamped = value.clamp(-limit, limit);
        (f64::from(clamped), clamped != value)
    };
    let (forward, c0) = clamp(velocity.vx, MAX_HORIZONTAL_MPS);
    let (right, c1) = clamp(velocity.vy, MAX_HORIZONTAL_MPS);
    let (down, c2) = clamp(velocity.vz, MAX_VERTICAL_MPS);
    let (yaw_rate, c3) = clamp(velocity.yaw_rate, MAX_YAW_RATE_RPS);
    let demand = Demand {
        forward,
        right,
        up: -down,
        yaw_rate,
    };
    (demand, c0 || c1 || c2 || c3)
}
