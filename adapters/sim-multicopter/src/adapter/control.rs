//! Control frames: discrete actions and typed intents per scope.

use pilotage_adapter_api::{ActionResult, ApplyOutcome, Disposition, RejectReason};
use pilotage_protocol::{ControlAction, ControlIntent, ReferenceFrame, ScopedControlFrame};
use pilotage_timing::SimTick;

use super::{GIMBAL_SCOPE, MOTION_SCOPE, SimMulticopterAdapter};
use crate::kinematics::BodyVelocity;

impl SimMulticopterAdapter {
    pub(super) fn apply(&mut self, frame: &ScopedControlFrame) -> ApplyOutcome {
        let tick = SimTick::new(self.tick);
        if frame.vehicle != self.vehicle {
            return ApplyOutcome::new(tick, Disposition::Rejected(RejectReason::UnknownVehicle));
        }
        let scope = frame.scope.as_str();
        if scope != MOTION_SCOPE && scope != GIMBAL_SCOPE {
            return ApplyOutcome::new(tick, Disposition::Rejected(RejectReason::UnknownScope));
        }
        if frame.carries_payload() {
            return ApplyOutcome::new(
                tick,
                Disposition::Rejected(RejectReason::Other(
                    "the simulated multicopter consumes typed intents only".to_owned(),
                )),
            );
        }
        if self.policy(scope).is_some() {
            return ApplyOutcome::new(tick, Disposition::Rejected(RejectReason::LinkLossEngaged));
        }
        let action_results: Vec<ActionResult> = frame
            .actions
            .iter()
            .map(|action| self.act(scope, *action))
            .collect();
        let disposition = match &frame.intent {
            None => Disposition::Accepted,
            Some(intent) if scope == MOTION_SCOPE => self.apply_motion(intent),
            Some(intent) => self.apply_gimbal(intent),
        };
        ApplyOutcome {
            tick,
            disposition,
            action_results,
        }
    }

    fn act(&mut self, scope: &str, action: ControlAction) -> ActionResult {
        let before = self.phase;
        let result = match (scope, action) {
            (MOTION_SCOPE, ControlAction::Arm) => self.phase.arm(),
            (MOTION_SCOPE, ControlAction::Disarm) => self.phase.disarm(),
            (MOTION_SCOPE, ControlAction::Takeoff) => {
                if self.battery.is_empty() {
                    Err("the battery is empty")
                } else {
                    self.phase.takeoff()
                }
            }
            (MOTION_SCOPE, ControlAction::Land) => self.phase.land(),
            (GIMBAL_SCOPE, ControlAction::GimbalRecenter) => {
                self.gimbal.recenter();
                Ok(())
            }
            _ => Err("the scope does not support this action"),
        };
        if self.phase != before {
            // A lifecycle change discards the previous stick command.
            self.motion = None;
            tracing::info!(
                ?action,
                from = before.mode_name(),
                to = self.phase.mode_name(),
                "simulated multicopter lifecycle"
            );
        }
        match result {
            Ok(()) => ActionResult::accepted(action),
            Err(detail) => ActionResult::rejected(action, detail),
        }
    }

    fn apply_motion(&mut self, intent: &ControlIntent) -> Disposition {
        let ControlIntent::Velocity(velocity) = intent else {
            return other("vehicle.motion accepts velocity intents only");
        };
        if velocity.frame != ReferenceFrame::BodyFrd {
            return other("vehicle.motion accepts the BODY-FRD frame only");
        }
        if let Err(detail) = self.phase.admits_velocity() {
            return other(detail);
        }
        let (command, clamped) = BodyVelocity {
            forward: f64::from(velocity.vx),
            right: f64::from(velocity.vy),
            down: f64::from(velocity.vz),
            yaw_rate: f64::from(velocity.yaw_rate),
        }
        .clamped(&self.config.limits);
        self.motion = Some((command, self.tick));
        if clamped {
            Disposition::Constrained
        } else {
            Disposition::Accepted
        }
    }

    fn apply_gimbal(&mut self, intent: &ControlIntent) -> Disposition {
        let ControlIntent::GimbalRate(rate) = intent else {
            return other("vehicle.gimbal accepts gimbal-rate intents only");
        };
        let limited = self.gimbal.command_rate(f64::from(rate.pitch_rate));
        self.gimbal_rate_at = Some(self.tick);
        // The yaw follows the body; a yaw rate cannot be executed.
        if limited || rate.yaw_rate != 0.0 {
            Disposition::Constrained
        } else {
            Disposition::Accepted
        }
    }
}

fn other(detail: &str) -> Disposition {
    Disposition::Rejected(RejectReason::Other(detail.to_owned()))
}
