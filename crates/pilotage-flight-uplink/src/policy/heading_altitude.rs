//! The heading and altitude frames of a flight controller that holds
//! heading, altitude, and airspeed with its own sensors.

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
