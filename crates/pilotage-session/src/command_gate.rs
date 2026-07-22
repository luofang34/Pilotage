//! The typed-command gate (CTRL-01): every accepted control frame passes
//! through here BEFORE adapter delivery, and comes out typed-only.
//!
//! The gate enforces exactly one command representation, validates the typed
//! command against the scope's advertised capability (family, reference
//! frame, magnitude limits, actions and mode targets, conflict-free action
//! sets), and holds the system's SINGLE legacy compatibility boundary: a
//! legacy numeric payload is translated into the scope's typed command using
//! the adapter-declared [`LegacyCommandMap`] and the same advertised limits a
//! typed client scales by. No numeric axis or button identifier crosses the
//! adapter boundary as meaning.

use pilotage_adapter_api::{IntentCapability, LegacyAxisRoute, LegacyCommandMap, ScopeDescriptor};
use pilotage_protocol::{
    ButtonEdge, ControlAction, ControlIntent, ControlPayload, FrameRejectionReason,
    GimbalRateIntent, IntentFamily, ScopedControlFrame, VelocityIntent, actions_conflict,
};

/// Validates `frame` against the scope's advertised capability and returns
/// its typed-only form: a typed frame validated as-is, a legacy frame
/// translated through the scope's declared map. The returned frame NEVER
/// carries a numeric payload.
///
/// # Errors
///
/// Returns the typed [`FrameRejectionReason`] the host echoes to the sender:
/// empty/dual representation, an unadvertised intent family or reference
/// frame, a component beyond the advertised limits, an unadvertised action
/// or mode target, or a repeated/conflicting action set.
pub fn gate_frame(
    frame: &ScopedControlFrame,
    scope: &ScopeDescriptor,
) -> Result<ScopedControlFrame, FrameRejectionReason> {
    match (frame.carries_payload(), frame.carries_typed()) {
        (true, true) => Err(FrameRejectionReason::DualCommand),
        (false, false) => Err(FrameRejectionReason::EmptyCommand),
        (false, true) => {
            validate_typed(frame, scope)?;
            let mut typed = frame.clone();
            typed.payload = ControlPayload::default();
            Ok(typed)
        }
        (true, false) => translate_legacy(frame, scope),
    }
}

fn validate_typed(
    frame: &ScopedControlFrame,
    scope: &ScopeDescriptor,
) -> Result<(), FrameRejectionReason> {
    if actions_conflict(&frame.actions) {
        return Err(FrameRejectionReason::ConflictingActions);
    }
    for action in &frame.actions {
        validate_action(*action, scope)?;
    }
    if let Some(intent) = &frame.intent {
        let Some(capability) = scope
            .intents
            .iter()
            .find(|capability| capability.family == intent.family())
        else {
            return Err(FrameRejectionReason::UnsupportedIntent);
        };
        if let Some(reference) = intent.reference_frame()
            && !capability.frames.contains(&reference)
        {
            return Err(FrameRejectionReason::UnsupportedIntent);
        }
        check_limits(intent, capability)?;
    }
    Ok(())
}

/// Magnitude limits are enforced against the scope's own advertisement, so
/// "within limits" means the same thing to the client that scaled by them,
/// this gate, and the adapter that declared them. A zero limit advertises no
/// bound and admits any finite value (finiteness is a decode guarantee).
fn check_limits(
    intent: &ControlIntent,
    capability: &IntentCapability,
) -> Result<(), FrameRejectionReason> {
    let within = |value: f32, limit: f32| limit == 0.0 || value.abs() <= limit;
    let ok = match intent {
        ControlIntent::Velocity(v) => {
            within(v.vx, capability.max_linear)
                && within(v.vy, capability.max_linear)
                && within(v.vz, capability.effective_vertical())
                && within(v.yaw_rate, capability.max_angular)
        }
        ControlIntent::PositionHold(p) => {
            within(p.x, capability.max_linear)
                && within(p.y, capability.max_linear)
                && within(p.z, capability.effective_vertical())
                && within(p.heading, capability.max_angular)
        }
        ControlIntent::BodyRate(b) => {
            within(b.roll_rate, capability.max_angular)
                && within(b.pitch_rate, capability.max_angular)
                && within(b.yaw_rate, capability.max_angular)
        }
        ControlIntent::GimbalRate(g) => {
            within(g.pitch_rate, capability.max_angular)
                && within(g.yaw_rate, capability.max_angular)
        }
        // Thrust bounds and quaternion unity are decode guarantees;
        // `max_angular` bounds the commanded TILT angle (yaw is a heading
        // setpoint, not a rate, and carries no advertised bound).
        ControlIntent::AttitudeThrust(a) => {
            let (roll, pitch, _) = pilotage_adapter_api::attitude_euler(a);
            within(roll, capability.max_angular) && within(pitch, capability.max_angular)
        }
    };
    if ok {
        Ok(())
    } else {
        Err(FrameRejectionReason::LimitExceeded)
    }
}

/// Validates one typed action against the scope's OWN advertisement: the
/// action kind must be advertised, and a mode request's target must be one
/// the vehicle listed. Shared by the frame gate and the reliable
/// action-command path (CTRL-01).
pub(crate) fn validate_action(
    action: ControlAction,
    scope: &ScopeDescriptor,
) -> Result<(), FrameRejectionReason> {
    let Some(capability) = scope
        .actions
        .iter()
        .find(|capability| capability.action == action.kind())
    else {
        return Err(FrameRejectionReason::UnsupportedAction);
    };
    if let ControlAction::ModeRequest { target } = action
        && !capability.mode_targets.contains(&target)
    {
        return Err(FrameRejectionReason::UnsupportedAction);
    }
    Ok(())
}

/// The single legacy compatibility boundary: interprets a numeric payload
/// through the scope's adapter-declared map into the typed command a typed
/// client would have sent, then validates the result through the SAME gate.
/// A scope that declares no map admits no legacy senders.
fn translate_legacy(
    frame: &ScopedControlFrame,
    scope: &ScopeDescriptor,
) -> Result<ScopedControlFrame, FrameRejectionReason> {
    let Some(map) = &scope.legacy else {
        return Err(FrameRejectionReason::UnsupportedIntent);
    };
    let mut typed = frame.clone();
    typed.payload = ControlPayload::default();
    let (intent, actions) = match map {
        LegacyCommandMap::Velocity { .. } => translate_velocity(frame, scope, map)?,
        LegacyCommandMap::GimbalRate { .. } => translate_gimbal_rate(frame, scope, map)?,
    };
    typed.intent = intent;
    typed.actions = actions;
    if !typed.carries_typed() {
        return Err(FrameRejectionReason::EmptyCommand);
    }
    validate_typed(&typed, scope)?;
    Ok(typed)
}

/// The velocity-map translation. An edges-only legacy frame carries no
/// velocity intent (alive traffic / discrete presses). A frame that reports
/// ANY axis must report EVERY routed one: the typed translation is
/// structurally total, so a partial payload would silently turn "no update"
/// into an explicit neutral — and could demonstrate a neutral activation the
/// sender never proved. An unmapped legacy button has no typed meaning;
/// dropping it is the fail-closed translation.
fn translate_velocity(
    frame: &ScopedControlFrame,
    scope: &ScopeDescriptor,
    map: &LegacyCommandMap,
) -> Result<(Option<ControlIntent>, Vec<ControlAction>), FrameRejectionReason> {
    let LegacyCommandMap::Velocity {
        vx,
        vy,
        vz,
        yaw_rate,
        arm_button,
        disarm_button,
    } = map
    else {
        return Err(FrameRejectionReason::UnsupportedIntent);
    };
    let capability = velocity_capability(scope)?;
    let intent = if frame.payload.axes.is_empty() {
        None
    } else {
        require_full_coverage(&frame.payload, &[*vx, *vy, *vz, *yaw_rate])?;
        Some(ControlIntent::Velocity(VelocityIntent {
            frame: *capability
                .frames
                .first()
                .ok_or(FrameRejectionReason::UnsupportedIntent)?,
            vx: routed(&frame.payload, *vx, capability.max_linear),
            vy: routed(&frame.payload, *vy, capability.max_linear),
            vz: routed(&frame.payload, *vz, capability.effective_vertical()),
            yaw_rate: routed(&frame.payload, *yaw_rate, capability.max_angular),
        }))
    };
    let actions = frame
        .payload
        .edges
        .iter()
        .filter(|(_, edge)| *edge == ButtonEdge::Pressed)
        .filter_map(|(button, _)| {
            let id = button.as_u16();
            if Some(id) == *arm_button {
                Some(ControlAction::Arm)
            } else if Some(id) == *disarm_button {
                Some(ControlAction::Disarm)
            } else {
                None
            }
        })
        .collect();
    Ok((intent, actions))
}

/// The gimbal-rate-map translation: a recenter-only legacy frame (no axes)
/// carries no rate intent; a frame that reports ANY axis must report every
/// routed one (the same total-translation rule as the velocity map).
fn translate_gimbal_rate(
    frame: &ScopedControlFrame,
    scope: &ScopeDescriptor,
    map: &LegacyCommandMap,
) -> Result<(Option<ControlIntent>, Vec<ControlAction>), FrameRejectionReason> {
    let LegacyCommandMap::GimbalRate {
        pitch,
        yaw,
        recenter_button,
    } = map
    else {
        return Err(FrameRejectionReason::UnsupportedIntent);
    };
    let capability = scope
        .intents
        .iter()
        .find(|capability| capability.family == IntentFamily::GimbalRate)
        .ok_or(FrameRejectionReason::UnsupportedIntent)?;
    let intent = if frame.payload.axes.is_empty() {
        None
    } else {
        require_full_coverage(&frame.payload, &[*pitch, *yaw])?;
        Some(ControlIntent::GimbalRate(GimbalRateIntent {
            pitch_rate: routed(&frame.payload, *pitch, capability.max_angular),
            yaw_rate: routed(&frame.payload, *yaw, capability.max_angular),
        }))
    };
    let actions = frame
        .payload
        .edges
        .iter()
        .filter(|(_, edge)| *edge == ButtonEdge::Pressed)
        .filter_map(|(button, _)| {
            (Some(button.as_u16()) == *recenter_button).then_some(ControlAction::GimbalRecenter)
        })
        .collect();
    Ok((intent, actions))
}

/// Every routed axis must appear in the payload; the deliberately absent
/// routes (`None`) require nothing.
fn require_full_coverage(
    payload: &ControlPayload,
    routes: &[Option<LegacyAxisRoute>],
) -> Result<(), FrameRejectionReason> {
    for route in routes.iter().flatten() {
        if !payload
            .axes
            .iter()
            .any(|(axis, _)| axis.as_u16() == route.axis)
        {
            return Err(FrameRejectionReason::PartialCommand);
        }
    }
    Ok(())
}

fn velocity_capability(scope: &ScopeDescriptor) -> Result<&IntentCapability, FrameRejectionReason> {
    scope
        .intents
        .iter()
        .find(|capability| capability.family == IntentFamily::Velocity)
        .ok_or(FrameRejectionReason::UnsupportedIntent)
}

/// One routed legacy component: the signed normalized axis value scaled by
/// the advertised limit. An unrouted component or absent axis reads zero;
/// values beyond full scale clamp so translation can never exceed what a
/// typed client could send.
fn routed(payload: &ControlPayload, route: Option<LegacyAxisRoute>, limit: f32) -> f32 {
    let Some(route) = route else {
        return 0.0;
    };
    let raw = payload
        .axes
        .iter()
        .find(|(axis, _)| axis.as_u16() == route.axis)
        .map_or(0.0, |(_, value)| *value);
    (raw * route.sign).clamp(-1.0, 1.0) * limit
}

#[cfg(test)]
mod tests;
