//! Normalized motion onto the advertised intent envelope.
//!
//! A stick is a demand in [-1, 1]; the vehicle negotiated what full
//! demand means. Scaling by the ADVERTISED envelope — and refusing to
//! build an intent without one — keeps a client from commanding what the
//! vehicle never offered. This is the same rule the browser applies; the
//! two clients share it by sharing this function's tests against the
//! same advertised numbers.

use pilotage_protocol::wire;

/// One normalized motion demand, each axis in [-1, 1]. Flight mapping:
/// pitch is forward, roll is right, positive throttle climbs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionDemand {
    /// Rightward demand.
    pub roll: f32,
    /// Forward demand.
    pub pitch: f32,
    /// Climb demand.
    pub throttle: f32,
    /// Right-yaw demand.
    pub yaw: f32,
}

/// Builds the typed velocity intent (m/s, rad/s) from a normalized
/// demand and the scope's advertised velocity capability. Returns `None`
/// without an advertisement: an unadvertised intent must not be sent.
#[must_use]
pub fn velocity_intent(
    demand: MotionDemand,
    capability: Option<&wire::IntentCapability>,
) -> Option<wire::ControlIntent> {
    let capability = capability?;
    if capability.family != wire::IntentFamily::Velocity as i32 {
        return None;
    }
    let max_vertical = if capability.max_vertical > 0.0 {
        capability.max_vertical
    } else {
        capability.max_linear
    };
    Some(wire::ControlIntent {
        family: Some(wire::control_intent::Family::Velocity(
            wire::VelocityIntent {
                frame: wire::ReferenceFrame::LocalNed as i32,
                vx: demand.pitch * capability.max_linear,
                vy: demand.roll * capability.max_linear,
                // Body-FRD +z is down; a climb demand is a negative vz.
                vz: -demand.throttle * max_vertical,
                yaw_rate: demand.yaw * capability.max_angular,
            },
        )),
    })
}

/// The advertised capability of `family` for `(vehicle, scope)` in an
/// admission catalog, or `None`. The vehicle participates in the match:
/// two vehicles may publish the same scope name with different envelopes.
#[must_use]
pub fn intent_capability<'a>(
    admission: &'a crate::Admission,
    vehicle_id: u64,
    scope: &str,
    family: wire::IntentFamily,
) -> Option<&'a wire::IntentCapability> {
    admission
        .vehicles
        .iter()
        .find(|vehicle| vehicle.vehicle_id == vehicle_id)?
        .scopes
        .iter()
        .find(|descriptor| descriptor.scope == scope)?
        .intents
        .iter()
        .find(|intent| intent.family == family as i32)
}
