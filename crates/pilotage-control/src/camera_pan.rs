//! Capability-based routing of camera pan to a gimbal or the airframe.

use pilotage_client_session::VehicleCatalog;
use pilotage_protocol::{payload_rate_limit, wire};

use crate::{GIMBAL_SCOPE, Mode};

/// The destination of the horizontal camera stick while the modifier is held.
/// This declaration does not grant either control lease.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum CameraPanRouting {
    /// Use the gimbal yaw axis, including hosts without per-axis declarations.
    #[default]
    GimbalYaw,
    /// Use airframe yaw through the selected flight scheme and motion lease.
    AirframeYaw,
    /// The gimbal has no yaw control and the flight scheme offers no usable yaw.
    AirframeYawUnavailable,
    /// No supported pan destination is known. Do not infer an airframe fallback.
    Unavailable,
}

/// Select camera pan from declared axis and flight capabilities.
///
/// Only an explicit absence of both gimbal yaw position and rate control permits
/// an airframe fallback. Temporary loss of gimbal control never changes its role.
#[must_use]
pub fn camera_pan_routing(vehicle: &VehicleCatalog, mode: Mode) -> CameraPanRouting {
    let Some(scope) = vehicle
        .scopes
        .iter()
        .find(|scope| scope.scope == GIMBAL_SCOPE)
    else {
        return CameraPanRouting::Unavailable;
    };
    if !mode.carries_gimbal()
        || !scope
            .intents
            .iter()
            .any(|intent| intent.family == wire::IntentFamily::GimbalRate as i32)
    {
        return CameraPanRouting::Unavailable;
    }
    let Some(payload) = &scope.payload else {
        return CameraPanRouting::GimbalYaw;
    };
    let Some(gimbal) = &payload.gimbal else {
        return CameraPanRouting::Unavailable;
    };
    if payload_rate_limit(gimbal.yaw.as_ref()).is_some() {
        return CameraPanRouting::GimbalYaw;
    }
    if payload_rate_limit(gimbal.pitch.as_ref()).is_none() || !yaw_unsupported(gimbal.yaw.as_ref())
    {
        return CameraPanRouting::Unavailable;
    }
    if airframe_yaw_available(vehicle, mode) {
        CameraPanRouting::AirframeYaw
    } else {
        CameraPanRouting::AirframeYawUnavailable
    }
}

fn yaw_unsupported(axis: Option<&wire::PayloadAxisCapabilities>) -> bool {
    let Some(axis) = axis else { return false };
    let unsupported = wire::PayloadSupportState::Unsupported as i32;
    axis.position
        .as_ref()
        .is_some_and(|position| position.support == unsupported)
        && axis
            .rate
            .as_ref()
            .is_some_and(|rate| rate.support == unsupported)
}

fn airframe_yaw_available(vehicle: &VehicleCatalog, mode: Mode) -> bool {
    let Some(choice) = crate::offered_schemes(vehicle).choice(mode).copied() else {
        return false;
    };
    vehicle
        .scopes
        .iter()
        .filter(|scope| scope.scope == choice.scope && !scope.command_only)
        .flat_map(|scope| &scope.intents)
        .filter(|intent| intent.family == choice.family as i32)
        .any(|intent| match choice.family {
            wire::IntentFamily::Velocity => {
                intent
                    .frames
                    .contains(&(wire::ReferenceFrame::BodyFrd as i32))
                    && positive(intent.max_angular)
            }
            wire::IntentFamily::AttitudeThrust => {
                intent
                    .frames
                    .contains(&(wire::ReferenceFrame::LocalNed as i32))
                    && positive(intent.max_yaw_rate)
            }
            wire::IntentFamily::BodyRate => positive(if intent.max_yaw_rate > 0.0 {
                intent.max_yaw_rate
            } else {
                intent.max_angular
            }),
            _ => false,
        })
}

fn positive(value: f32) -> bool {
    value.is_finite() && value > 0.0
}

#[cfg(test)]
mod tests;
