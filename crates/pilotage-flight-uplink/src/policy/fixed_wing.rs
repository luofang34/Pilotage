//! The frames of a fixed wing, whose flight controller holds a course or
//! a heading, an altitude or a climb rate, and an airspeed with its own
//! sensors. None of them goes through the multirotor stick law: a fixed
//! wing has no hover to brake to and no climb input that opens its stream.

use std::time::Duration;

use pilotage_mavlink::codec::{encode_airspeed_command, encode_heading_altitude_setpoint};

use super::{HeadingState, UplinkPolicy, boot_ms};

/// How often an unchanged airspeed command is sent again.
pub const AIRSPEED_REPEAT: Duration = Duration::from_secs(1);

/// One heading and altitude demand, already in the flight controller's
/// local frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeadingAltitudeDemand {
    /// The down coordinate of the altitude in the local frame, metres.
    pub down_m: f32,
    /// The true heading, radians.
    pub heading_rad: f32,
    /// The indicated airspeed to hold, m/s; `None` leaves the one the
    /// flight controller holds now.
    pub airspeed_mps: Option<f32>,
}

impl UplinkPolicy {
    /// The velocity setpoint of a fixed wing, in the local NED frame, with
    /// the yaw the flight controller holds when the velocity has no
    /// horizontal direction. Nothing inside the post-arm quiet interval.
    ///
    /// The flight controller reads the horizontal velocity as a course and
    /// an airspeed and the down component as a sink rate; the caller has
    /// already bounded each by the airframe's envelope.
    pub fn fixed_wing_velocity_frame(
        &mut self,
        now: Duration,
        velocity_ned_mps: [f32; 3],
        yaw_rad: f32,
    ) -> Option<Vec<u8>> {
        if self.in_quiet_interval(now) {
            return None;
        }
        self.airborne = true;
        self.hold_started = None;
        self.last_hold = None;
        self.heading = HeadingState {
            heading_rad: yaw_rad,
            valid: yaw_rad.is_finite(),
        };
        Some(self.velocity_frame(now, velocity_ned_mps))
    }

    /// The frames of one heading and altitude demand: the airspeed command
    /// when it is due, then the setpoint. Nothing inside the post-arm quiet
    /// interval, whose frames would overwrite the arm in the flight
    /// controller's command slot.
    ///
    /// The airspeed goes in its own command, never in the setpoint's
    /// velocity: a velocity there is a ground-referenced demand that the
    /// flight controller would fly instead of the heading.
    pub fn heading_altitude_frames(
        &mut self,
        now: Duration,
        demand: HeadingAltitudeDemand,
    ) -> Vec<Vec<u8>> {
        if self.in_quiet_interval(now) {
            return Vec::new();
        }
        // A heading and altitude is a demand to fly, so it opens the
        // stream that the motors-idle gate keeps closed for a multirotor.
        self.airborne = true;
        self.hold_started = None;
        self.last_hold = None;
        self.heading = HeadingState {
            heading_rad: demand.heading_rad,
            valid: true,
        };
        let mut frames = Vec::with_capacity(2);
        if let Some(airspeed) = demand.airspeed_mps
            && self.airspeed_due(now, airspeed)
        {
            frames.push(
                encode_airspeed_command(self.seq, airspeed, self.system_id, self.component_id)
                    .to_vec(),
            );
            self.seq = self.seq.wrapping_add(1);
            self.airspeed_sent = Some((airspeed, now));
        }
        frames.push(
            encode_heading_altitude_setpoint(
                self.seq,
                boot_ms(now),
                demand.down_m,
                demand.heading_rad,
                self.system_id,
                self.component_id,
            )
            .to_vec(),
        );
        self.seq = self.seq.wrapping_add(1);
        frames
    }

    /// Whether the airspeed command goes out with this demand: a new
    /// airspeed, or the same one again once [`AIRSPEED_REPEAT`] has passed.
    /// The command travels on a link that can lose it, and the setpoint
    /// beside it does not carry the airspeed, so only a repeat recovers a
    /// lost one.
    fn airspeed_due(&self, now: Duration, airspeed_mps: f32) -> bool {
        self.airspeed_sent.is_none_or(|(sent, at)| {
            sent != airspeed_mps || now.saturating_sub(at) >= AIRSPEED_REPEAT
        })
    }
}

#[cfg(test)]
mod tests;
