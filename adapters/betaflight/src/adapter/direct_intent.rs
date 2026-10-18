//! The direct scope's typed intents as direct demands, clamped to the
//! envelope it advertises.

use pilotage_protocol::{AttitudeThrustIntent, BodyRateIntent, ControlIntent};

use crate::config::{MAX_BODY_RATE_RPS, MAX_TILT_RAD};
use crate::direct::DirectDemand;

/// The demand for a body-rate or attitude intent, and whether it was
/// clamped; `None` for any other family.
pub(super) fn demand(intent: &ControlIntent) -> Option<(DirectDemand, bool)> {
    match intent {
        ControlIntent::BodyRate(rates) => Some(from_rates(rates)),
        ControlIntent::AttitudeThrust(attitude) => Some(from_attitude(attitude)),
        _ => None,
    }
}

/// `value` inside `±limit`, and whether that changed it. Non-finite reads
/// as zero.
fn clamp(value: f64, limit: f64) -> (f64, bool) {
    let value = if value.is_finite() { value } else { 0.0 };
    let clamped = value.clamp(-limit, limit);
    (clamped, clamped != value)
}

fn thrust(value: f32) -> (f64, bool) {
    let (centred, changed) = clamp(f64::from(value) - 0.5, 0.5);
    (centred + 0.5, changed)
}

fn from_rates(rates: &BodyRateIntent) -> (DirectDemand, bool) {
    let limit = f64::from(MAX_BODY_RATE_RPS);
    let (roll, c0) = clamp(f64::from(rates.roll_rate), limit);
    let (pitch, c1) = clamp(f64::from(rates.pitch_rate), limit);
    let (yaw, c2) = clamp(f64::from(rates.yaw_rate), limit);
    let (thrust, c3) = thrust(rates.thrust);
    (
        DirectDemand::Rates {
            roll,
            pitch,
            yaw,
            thrust,
        },
        c0 || c1 || c2 || c3,
    )
}

/// The Z-Y-X roll, pitch, and heading of the body-to-NED quaternion; the
/// tilt is clamped per axis to the advertised bound.
fn from_attitude(attitude: &AttitudeThrustIntent) -> (DirectDemand, bool) {
    let [w, x, y, z] = [attitude.qw, attitude.qx, attitude.qy, attitude.qz].map(f64::from);
    let norm = (w * w + x * x + y * y + z * z).sqrt();
    let [w, x, y, z] = if norm.is_finite() && norm > 1e-6 {
        [w, x, y, z].map(|c| c / norm)
    } else {
        [1.0, 0.0, 0.0, 0.0]
    };
    let roll = (2.0 * (w * x + y * z)).atan2(1.0 - 2.0 * (x * x + y * y));
    let pitch = (2.0 * (w * y - z * x)).clamp(-1.0, 1.0).asin();
    let heading = (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z));
    let limit = f64::from(MAX_TILT_RAD);
    let (roll, c0) = clamp(roll, limit);
    let (pitch, c1) = clamp(pitch, limit);
    let (thrust, c2) = thrust(attitude.thrust);
    (
        DirectDemand::Attitude {
            roll,
            pitch,
            heading,
            thrust,
        },
        c0 || c1 || c2,
    )
}
