//! Flight-scope frame handling: structural checks, the discrete actions,
//! and the velocity envelope.

use std::time::Instant;

use pilotage_adapter_api::{ApplyOutcome, Disposition, RejectReason};
use pilotage_protocol::{
    ActionKind, ControlIntent, LogicalAxisId, ScopedControlFrame, VelocityIntent,
};
use pilotage_timing::SimTick;

use super::{
    ArduPilotAdapter, FLIGHT_SCOPE, PITCH_AXIS, ROLL_AXIS, THROTTLE_AXIS, YAW_AXIS, action_results,
};
use crate::uplink::{MAX_HORIZONTAL_MPS, MAX_VERTICAL_MPS, MAX_YAW_RATE_RPS, VelocityOutcome};

fn rejected(tick: SimTick, reason: RejectReason) -> ApplyOutcome {
    ApplyOutcome::new(tick, Disposition::Rejected(reason))
}

/// Clamps a body-frame velocity intent to the advertised envelope:
/// (forward, right, down, yaw rate) and whether anything was clamped. A
/// NaN component becomes zero.
pub(super) fn clamp_to_envelope(velocity: &VelocityIntent) -> ([f32; 4], bool) {
    let clamp = |value: f32, limit: f32| {
        let clamped = if value.is_nan() {
            0.0
        } else {
            value.clamp(-limit, limit)
        };
        (clamped, clamped != value)
    };
    let (vx, c0) = clamp(velocity.vx, MAX_HORIZONTAL_MPS);
    let (vy, c1) = clamp(velocity.vy, MAX_HORIZONTAL_MPS);
    let (vz, c2) = clamp(velocity.vz, MAX_VERTICAL_MPS);
    let (yaw_rate, c3) = clamp(velocity.yaw_rate, MAX_YAW_RATE_RPS);
    ([vx, vy, vz, yaw_rate], c0 || c1 || c2 || c3)
}

fn disposition(outcome: VelocityOutcome) -> Disposition {
    match outcome {
        VelocityOutcome::Streamed { constrained: false } => Disposition::Accepted,
        // The intent became a clamped setpoint, or a takeoff.
        VelocityOutcome::Streamed { constrained: true } | VelocityOutcome::TakeoffCommanded => {
            Disposition::Transformed
        }
        // The FC owns the motion for now: takeoff climb, landing, or a
        // landed vehicle with no climb demand.
        VelocityOutcome::TakeoffInProgress
        | VelocityOutcome::Landing
        | VelocityOutcome::HeldOnGround => Disposition::Constrained,
        VelocityOutcome::NotArmed => Disposition::Rejected(RejectReason::UplinkIdle),
        VelocityOutcome::LaneClosed => Disposition::Rejected(RejectReason::Other(
            "the ArduPilot setpoint lane is closed".to_owned(),
        )),
    }
}

impl ArduPilotAdapter {
    fn validate_flight_frame(&self, frame: &ScopedControlFrame) -> Result<(), RejectReason> {
        if frame.vehicle != self.vehicle {
            return Err(RejectReason::UnknownVehicle);
        }
        if frame.scope.as_str() != FLIGHT_SCOPE {
            return Err(RejectReason::UnknownScope);
        }
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
        Ok(())
    }

    /// Applies one flight-scope frame. Disarm short-circuits the frame;
    /// arm goes to the FC whatever the estimate says, because ArduCopter's
    /// pre-arm checks decide and say why. A velocity intent needs a
    /// current pose: the takeoff phase is judged on the estimated height.
    pub(super) fn apply_flight(
        &mut self,
        frame: &ScopedControlFrame,
        tick: SimTick,
    ) -> ApplyOutcome {
        if let Err(reason) = self.validate_flight_frame(frame) {
            return rejected(tick, reason);
        }
        let armed = self.fc_armed();
        let pose = self.current_pose();
        let Some(uplink) = self.uplink.as_mut() else {
            return rejected(tick, RejectReason::UnknownScope);
        };
        let now = Instant::now();
        if frame
            .actions
            .iter()
            .any(|action| action.kind() == ActionKind::Disarm)
        {
            let outcome = uplink.disarm(now);
            tracing::info!(?outcome, "disarm action");
            return ApplyOutcome {
                tick,
                disposition: Disposition::Accepted,
                action_results: action_results(frame, &[ActionKind::Disarm]),
            };
        }
        if frame
            .actions
            .iter()
            .any(|action| action.kind() == ActionKind::Arm)
        {
            uplink.arm(armed, now);
        }
        let action_results = action_results(frame, &[ActionKind::Arm]);
        let Some(ControlIntent::Velocity(velocity)) = frame.intent else {
            return ApplyOutcome {
                tick,
                disposition: Disposition::Accepted,
                action_results,
            };
        };
        let Some(pose) = pose else {
            return ApplyOutcome {
                tick,
                disposition: Disposition::Rejected(RejectReason::MeasurementUnavailable),
                action_results,
            };
        };
        let (demand, constrained) = clamp_to_envelope(&velocity);
        let outcome = uplink.apply_velocity(demand, constrained, armed, pose.pos_ned_m[2], now);
        ApplyOutcome {
            tick,
            disposition: disposition(outcome),
            action_results,
        }
    }
}
