//! VFR_HUD: the airspeed a flight controller with an airspeed probe
//! measures. Its other values duplicate the estimate groups, which carry
//! them with their own validity, so only the airspeed is read.

/// VFR_HUD message id.
pub const VFR_HUD_ID: u32 = 74;

/// The airspeed one VFR_HUD states. The message has no timestamp of its
/// own.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VfrHud {
    /// Indicated airspeed, m/s; `None` when the sender states none (a NaN
    /// on the wire, which a flight controller without a probe sends) or
    /// the value is negative.
    pub indicated_airspeed_mps: Option<f32>,
}

impl VfrHud {
    /// Payload: airspeed f32 @0, then groundspeed, altitude, climb,
    /// heading, and throttle, none of which is read.
    pub(super) fn decode(payload: &[u8]) -> Self {
        let airspeed = super::decoding::f32_at(payload, 0);
        Self {
            indicated_airspeed_mps: (airspeed.is_finite() && airspeed >= 0.0).then_some(airspeed),
        }
    }
}
