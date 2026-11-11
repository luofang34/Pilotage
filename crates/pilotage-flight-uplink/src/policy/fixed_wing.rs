//! The frames of a fixed wing, whose flight controller holds a course or
//! a heading, an altitude or a climb rate, and an airspeed with its own
//! sensors. None of them goes through the multirotor stick law: a fixed
//! wing has no hover to brake to and no climb input that opens its stream.

use std::time::Duration;

use pilotage_mavlink::codec::{encode_airspeed_command, encode_heading_altitude_setpoint};

use super::{HeadingState, UplinkPolicy, boot_ms};

/// How often an unchanged airspeed command the flight controller has not
/// accepted is sent again.
pub const AIRSPEED_REPEAT: Duration = Duration::from_secs(1);

/// What a flight controller's answer to the airspeed command changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AirspeedAnswer {
    /// The airspeed is set; it is not sent again until it changes.
    Accepted,
    /// Not set this time (temporarily rejected, failed, in progress); it
    /// is sent again at [`AIRSPEED_REPEAT`].
    Retry,
    /// The flight controller refuses airspeed targets and will keep doing
    /// so: the command is not sent again, and the vehicle flies at its own
    /// cruise airspeed. Returned once, when the refusal first arrives.
    Refused,
    /// The answer changes nothing: no command is out, or the refusal is
    /// already known.
    Ignored,
}

/// The airspeed command's state: what was last sent, whether the flight
/// controller accepted it, and whether it refuses airspeed targets.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct AirspeedCommand {
    /// The airspeed last sent, m/s, and when.
    last: Option<(f32, Duration)>,
    /// Whether the flight controller accepted the airspeed last sent.
    accepted: bool,
    /// Whether the flight controller refuses airspeed targets for good. An
    /// arm keeps it: the refusal is the flight controller's, not the
    /// flight's.
    pub(super) refused: bool,
}

impl AirspeedCommand {
    /// Whether the command goes out with a demand for `airspeed_mps`: never
    /// once refused; a new airspeed at once; the same one again only while
    /// it is not accepted, once [`AIRSPEED_REPEAT`] has passed. The command
    /// travels on a link that can lose it, and the setpoint beside it does
    /// not carry the airspeed, so only a repeat recovers a lost one.
    pub(super) fn due(&self, now: Duration, airspeed_mps: f32) -> bool {
        if self.refused {
            return false;
        }
        self.last.is_none_or(|(sent, at)| {
            sent != airspeed_mps || (!self.accepted && now.saturating_sub(at) >= AIRSPEED_REPEAT)
        })
    }

    /// Records the command sent for `airspeed_mps` at `now`.
    pub(super) fn sent(&mut self, airspeed_mps: f32, now: Duration) {
        if self.last.is_none_or(|(sent, _)| sent != airspeed_mps) {
            self.accepted = false;
        }
        self.last = Some((airspeed_mps, now));
    }

    /// Forgets what was sent, after an arm: the flight controller may then
    /// hold another airspeed.
    pub(super) fn forget_sent(&mut self) {
        self.last = None;
        self.accepted = false;
    }

    /// Takes the flight controller's answer, as a MAVLink result code.
    pub(super) fn answered(&mut self, mav_result: u8) -> AirspeedAnswer {
        if self.last.is_none() || self.refused {
            return AirspeedAnswer::Ignored;
        }
        match pilotage_mavlink::command_outcome(mav_result) {
            pilotage_adapter_api::CommandOutcome::Accepted => {
                self.accepted = true;
                AirspeedAnswer::Accepted
            }
            pilotage_adapter_api::CommandOutcome::Unsupported
            | pilotage_adapter_api::CommandOutcome::Denied => {
                self.refused = true;
                AirspeedAnswer::Refused
            }
            _ => AirspeedAnswer::Retry,
        }
    }
}

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
        self.heading = HeadingState {
            heading_rad: demand.heading_rad,
            valid: true,
        };
        let mut frames = Vec::with_capacity(2);
        if let Some(airspeed) = demand.airspeed_mps
            && self.airspeed.due(now, airspeed)
        {
            frames.push(
                encode_airspeed_command(self.seq, airspeed, self.system_id, self.component_id)
                    .to_vec(),
            );
            self.seq = self.seq.wrapping_add(1);
            self.airspeed.sent(airspeed, now);
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

    /// Takes the flight controller's answer to the airspeed command, as a
    /// MAVLink result code. A refusal that will not change (the command is
    /// unsupported, or denied) stops the command for good; see
    /// [`AirspeedAnswer`].
    pub fn airspeed_answered(&mut self, mav_result: u8) -> AirspeedAnswer {
        self.airspeed.answered(mav_result)
    }

    /// Whether the flight controller refused airspeed targets for good, so
    /// it flies every heading and altitude at its own cruise airspeed.
    #[must_use]
    pub const fn airspeed_refused(&self) -> bool {
        self.airspeed.refused
    }
}

#[cfg(test)]
mod tests;
