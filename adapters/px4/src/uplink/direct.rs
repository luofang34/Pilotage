//! Direct flight on the multicopter offboard stream: body rates or an
//! attitude, each with collective thrust, sent as SET_ATTITUDE_TARGET.
//!
//! PX4 offboard picks its controller from the setpoint type, so the stream
//! carries exactly one kind at a time; the keepalive resends the current
//! kind, never a stale velocity.
//!
//! Thrust takeover: a direct intent carries absolute normalized thrust, and
//! the operator's first value is arbitrary. Entering direct flight therefore
//! holds the thrust PX4 commands at that moment (its ATTITUDE_TARGET
//! report, the hover thrust in a hover) and hands thrust to the operator
//! only once their demand reaches that value. So a family switch never
//! steps the thrust, the way an RC throttle takeover works.

use pilotage_mavlink::codec::{
    AttitudeTarget, BodyRateTarget, encode_attitude_setpoint, encode_body_rate_setpoint,
};

use super::Px4Uplink;

/// The body-rate bound the uplink clamps at, rad/s (about 200°/s).
pub(crate) const MAX_BODY_RATE_RPS: f32 = 3.5;
/// The tilt bound an attitude demand is clamped at, rad (35°).
pub(crate) const MAX_TILT_RAD: f32 = 0.61;
/// How close the operator's thrust must come to the held thrust to take it
/// over without crossing it.
const TAKEOVER_BAND: f32 = 0.02;

/// One direct demand without its thrust.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum DirectDemand {
    /// Body rates (roll, pitch, yaw), rad/s, body FRD.
    BodyRate([f32; 3]),
    /// Attitude as roll, pitch, yaw Euler angles, rad, from local NED.
    Attitude([f32; 3]),
}

/// What became of a direct demand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DirectDisposition {
    /// Sent as commanded.
    Flown,
    /// Sent after clamping, or with the thrust still held for takeover.
    Constrained,
    /// No arm sequence is active, so nothing left the process.
    UplinkIdle,
    /// PX4 has not reported its commanded thrust yet, so there is no thrust
    /// to start direct flight from.
    ThrustUnknown,
}

/// The thrust handover from PX4's own value to the operator's.
#[derive(Debug, Clone, Copy)]
struct Takeover {
    held: f32,
    last_demand: Option<f32>,
    handed_over: bool,
}

impl Takeover {
    fn thrust(&mut self, demand: f32) -> f32 {
        if !self.handed_over {
            let reached = (demand - self.held).abs() <= TAKEOVER_BAND;
            let crossed = self
                .last_demand
                .is_some_and(|last| (last - self.held) * (demand - self.held) < 0.0);
            self.handed_over = reached || crossed;
            self.last_demand = Some(demand);
        }
        if self.handed_over { demand } else { self.held }
    }
}

/// The direct-flight state of the stream.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DirectState {
    // The latest collective thrust PX4 reports commanding.
    observed_thrust: Option<f32>,
    // The demand on the stream while direct flight is active.
    active: Option<(DirectDemand, Takeover, f32)>,
}

impl DirectState {
    pub(super) fn observe_thrust(&mut self, thrust: f32) {
        if thrust.is_finite() {
            self.observed_thrust = Some(thrust.clamp(0.0, 1.0));
        }
    }
}

impl Px4Uplink {
    /// Sends one direct demand, switching the stream to it. Rates and tilt
    /// beyond the advertised bounds clamp.
    pub(crate) fn send_direct(&mut self, demand: DirectDemand, thrust: f32) -> DirectDisposition {
        if self.arm_phase == super::ArmPhase::Idle {
            return DirectDisposition::UplinkIdle;
        }
        let (demand, clamped) = clamp_demand(demand);
        let mut takeover = match self.direct.active {
            Some((_, takeover, _)) => takeover,
            None => {
                let Some(held) = self.direct.observed_thrust else {
                    return DirectDisposition::ThrustUnknown;
                };
                tracing::info!(
                    held,
                    "direct flight: thrust held until the operator takes it over"
                );
                Takeover {
                    held,
                    last_demand: None,
                    handed_over: false,
                }
            }
        };
        let sent = takeover.thrust(if thrust.is_finite() { thrust } else { 0.0 }.clamp(0.0, 1.0));
        self.direct.active = Some((demand, takeover, sent));
        self.send_direct_setpoint();
        if clamped || !takeover.handed_over {
            DirectDisposition::Constrained
        } else {
            DirectDisposition::Flown
        }
    }

    /// Leaves direct flight for the velocity stream, seeding the velocity
    /// slew and heading from the measured state so the first velocity
    /// setpoint continues the motion instead of stepping to a stale one.
    pub(crate) fn enter_velocity(&mut self, yaw_rad: f32, vel_ned_mps: Option<[f32; 3]>) {
        if self.direct.active.take().is_some() {
            tracing::info!("direct flight ended; velocity stream resumes from the measured state");
            self.heading_sp_rad = yaw_rad;
            self.last_vel_ned = vel_ned_mps.unwrap_or([0.0; 3]);
        }
    }

    /// Drops any direct demand, for the paths that restart or stop the
    /// stream.
    pub(super) fn leave_direct(&mut self) {
        self.direct.active = None;
    }

    /// The keepalive: the current kind's latest setpoint.
    pub(super) fn send_keepalive(&mut self) {
        if self.direct.active.is_some() {
            self.send_direct_setpoint();
        } else {
            let velocity = self.last_vel_ned;
            self.send_velocity(velocity);
        }
    }

    fn send_direct_setpoint(&mut self) {
        let Some((demand, _, thrust)) = self.direct.active else {
            return;
        };
        self.last_frame = Some(self.clock.now());
        let time_boot_ms = self.time_boot_ms();
        let (system_id, component_id) = (self.expected_system_id, self.expected_component_id);
        match demand {
            DirectDemand::BodyRate([roll_rps, pitch_rps, yaw_rps]) => {
                let target = BodyRateTarget {
                    roll_rps,
                    pitch_rps,
                    yaw_rps,
                    thrust,
                    system_id,
                    component_id,
                };
                let frame = encode_body_rate_setpoint(self.seq, time_boot_ms, target);
                self.send(&frame);
            }
            DirectDemand::Attitude([roll_rad, pitch_rad, yaw_rad]) => {
                let target = AttitudeTarget {
                    roll_rad,
                    pitch_rad,
                    yaw_rad,
                    thrust,
                    system_id,
                    component_id,
                };
                let frame = encode_attitude_setpoint(self.seq, time_boot_ms, target);
                self.send(&frame);
            }
        }
    }

    /// The thrust the stream sends now, for tests.
    #[cfg(test)]
    pub(crate) fn direct_thrust(&self) -> Option<f32> {
        self.direct.active.map(|(_, _, thrust)| thrust)
    }
}

/// Clamps rates to [`MAX_BODY_RATE_RPS`] and roll and pitch to
/// [`MAX_TILT_RAD`]; NaN components become zero. Reports whether it clamped.
fn clamp_demand(demand: DirectDemand) -> (DirectDemand, bool) {
    let mut clamped = false;
    let mut bound = |value: f32, limit: f32| {
        let bounded = if value.is_nan() {
            0.0
        } else {
            value.clamp(-limit, limit)
        };
        clamped |= bounded != value;
        bounded
    };
    let demand = match demand {
        DirectDemand::BodyRate(rates) => {
            DirectDemand::BodyRate(rates.map(|rate| bound(rate, MAX_BODY_RATE_RPS)))
        }
        DirectDemand::Attitude([roll, pitch, yaw]) => DirectDemand::Attitude([
            bound(roll, MAX_TILT_RAD),
            bound(pitch, MAX_TILT_RAD),
            bound(yaw, core::f32::consts::PI),
        ]),
    };
    (demand, clamped)
}

/// Roll, pitch, yaw (ZYX Euler) of a unit quaternion `(w, x, y, z)`.
#[must_use]
pub(crate) fn euler_of(q: [f32; 4]) -> [f32; 3] {
    let [w, x, y, z] = q;
    let roll = (2.0 * (w * x + y * z)).atan2(1.0 - 2.0 * (x * x + y * y));
    let pitch = (2.0 * (w * y - z * x)).clamp(-1.0, 1.0).asin();
    let yaw = (2.0 * (w * z + x * y)).atan2(1.0 - 2.0 * (y * y + z * z));
    [roll, pitch, yaw]
}

#[cfg(test)]
mod tests;
