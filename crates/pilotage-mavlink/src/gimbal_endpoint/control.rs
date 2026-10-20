use std::time::Duration;

use pilotage_adapter_api::{ActionResult, ApplyOutcome, Disposition, RejectReason};
use pilotage_protocol::{
    ActionKind, ControlIntent, GimbalRateIntent, ScopedControlFrame, VehicleId,
};
use pilotage_timing::SimTick;

use crate::ManagerControl;
use crate::gimbal_control::{MAX_PITCH_RATE_RPS, MAX_YAW_RATE_RPS};

const CLAIM_DENIAL_FRESHNESS: Duration = Duration::from_secs(5);

/// Admit a typed pointing frame before any manager claim or motion command.
///
/// The caller must enforce its scope lease and link-loss latch first.
pub fn apply_control(
    control: &mut ManagerControl,
    vehicle: VehicleId,
    frame: &ScopedControlFrame,
    tick: SimTick,
) -> ApplyOutcome {
    if let Err(reason) = validate(vehicle, frame) {
        return rejected(frame, tick, reason);
    }
    control.refresh_discovery();
    if let Some(result) = control.claim_denial_cached(CLAIM_DENIAL_FRESHNESS) {
        return rejected(
            frame,
            tick,
            RejectReason::Other(format!(
                "gimbal primary-control claim denied by the manager (MAV_RESULT {result})"
            )),
        );
    }
    control.bind_timing(frame.admitted_timing);
    let (delivered, constrained) = if !frame.actions.is_empty() {
        (control.neutral(), false)
    } else if let Some(ControlIntent::GimbalRate(rate)) = frame.intent {
        let (pitch, yaw, constrained) = normalized_rate_demand(rate);
        (control.rate_demand(pitch, yaw), constrained)
    } else {
        (false, false)
    };
    if !delivered {
        return rejected(
            frame,
            tick,
            RejectReason::Other(control.rejection_detail().to_owned()),
        );
    }
    ApplyOutcome {
        tick,
        disposition: if constrained {
            Disposition::Constrained
        } else {
            Disposition::Accepted
        },
        action_results: frame
            .actions
            .iter()
            .copied()
            .map(ActionResult::accepted)
            .collect(),
    }
}

fn validate(vehicle: VehicleId, frame: &ScopedControlFrame) -> Result<(), RejectReason> {
    if frame.vehicle != vehicle {
        return Err(RejectReason::UnknownVehicle);
    }
    if frame.scope.as_str() != super::SCOPE {
        return Err(RejectReason::UnknownScope);
    }
    let invalid = |detail: &str| RejectReason::Other(detail.to_owned());
    if frame.carries_payload() || !frame.carries_typed() {
        return Err(invalid("the gimbal scope consumes typed commands only"));
    }
    if frame.actions.len() > 1
        || frame
            .actions
            .iter()
            .any(|action| action.kind() != ActionKind::GimbalRecenter)
    {
        return Err(invalid(
            "the gimbal scope accepts one recenter action per frame",
        ));
    }
    if let Some(intent) = frame.intent {
        let ControlIntent::GimbalRate(rate) = intent else {
            return Err(invalid(
                "the gimbal scope consumes gimbal-rate intents only",
            ));
        };
        if !rate.pitch_rate.is_finite() || !rate.yaw_rate.is_finite() {
            return Err(invalid("gimbal rates must be finite"));
        }
    }
    Ok(())
}

fn rejected(frame: &ScopedControlFrame, tick: SimTick, reason: RejectReason) -> ApplyOutcome {
    ApplyOutcome {
        tick,
        disposition: Disposition::Rejected(reason),
        action_results: frame
            .actions
            .iter()
            .map(|action| ActionResult::rejected(*action, "gimbal frame rejected"))
            .collect(),
    }
}

fn normalized_rate_demand(rate: GimbalRateIntent) -> (f32, f32, bool) {
    let normalize = |value: f32, limit: f32| {
        let normalized = value / limit;
        let clamped = normalized.clamp(-1.0, 1.0);
        (clamped, clamped != normalized)
    };
    let (pitch, pitch_clamped) = normalize(rate.pitch_rate, MAX_PITCH_RATE_RPS);
    let (yaw, yaw_clamped) = normalize(rate.yaw_rate, MAX_YAW_RATE_RPS);
    (pitch, yaw, pitch_clamped || yaw_clamped)
}
