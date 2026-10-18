//! Direct ("racing") flight: body-rate or attitude demands with collective
//! thrust, passed to Betaflight as sticks with no host loop around them.
//!
//! Body rates fly ACRO: Betaflight's rate curve is linear in this
//! configuration, so a rate is the stick deflection times
//! [`MAX_RATE_DPS`]. An attitude flies ANGLE: tilt is the deflection times
//! [`ANGLE_LIMIT_DEG`], and since ANGLE mode yaws at a rate, the heading
//! is held by a proportional rate toward the demanded one.
//!
//! Entering direct flight from assisted flight must not drop or jump the
//! quad, so the throttle stays at what assisted flight last sent until the
//! operator's thrust reaches it, then follows the operator.

use crate::rc::{ANGLE_LIMIT_DEG, MAX_RATE_DPS, Sticks};

/// Heading-hold gain for attitude flight, (rad/s)/rad.
const HEADING_GAIN: f64 = 2.0;
/// The thrust band in which the operator's thrust takes over the held one.
const THRUST_CAPTURE: f64 = 0.05;

/// One direct demand, in FRD/NED radians and normalized thrust.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum DirectDemand {
    /// Body rates (p, q, r), rad/s, and thrust 0..1.
    Rates {
        /// Roll rate, positive right wing down.
        roll: f64,
        /// Pitch rate, positive nose up.
        pitch: f64,
        /// Yaw rate, positive clockwise.
        yaw: f64,
        /// Collective thrust, 0..1.
        thrust: f64,
    },
    /// Roll, pitch, heading (Z-Y-X of the body-to-NED rotation), radians,
    /// and thrust 0..1.
    Attitude {
        /// Roll, positive right wing down.
        roll: f64,
        /// Pitch, positive nose up.
        pitch: f64,
        /// Heading, clockwise from north.
        heading: f64,
        /// Collective thrust, 0..1.
        thrust: f64,
    },
}

impl DirectDemand {
    fn thrust(&self) -> f64 {
        match *self {
            Self::Rates { thrust, .. } | Self::Attitude { thrust, .. } => thrust,
        }
    }
}

/// The throttle hand-over between assisted and direct flight.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct DirectLoop {
    held: Option<f64>,
}

impl DirectLoop {
    /// Direct flight starts with the throttle assisted flight last sent.
    pub(crate) fn begin(&mut self, throttle: f64) {
        self.held = Some(throttle);
    }

    /// The sticks for `demand` given the current `heading` (radians).
    pub(crate) fn sticks(&mut self, demand: DirectDemand, heading: f64) -> Sticks {
        let wanted = demand.thrust().clamp(0.0, 1.0);
        if self
            .held
            .is_some_and(|held| (wanted - held).abs() <= THRUST_CAPTURE)
        {
            self.held = None;
        }
        let throttle = self.held.unwrap_or(wanted);
        match demand {
            DirectDemand::Rates {
                roll, pitch, yaw, ..
            } => Sticks {
                roll: rate_stick(roll),
                // Betaflight's pitch stick forward is nose down.
                pitch: rate_stick(-pitch),
                throttle,
                yaw: rate_stick(yaw),
                arm: false,
                angle: false,
            },
            DirectDemand::Attitude {
                roll,
                pitch,
                heading: wanted_heading,
                ..
            } => Sticks {
                roll: roll.to_degrees() / ANGLE_LIMIT_DEG,
                pitch: -pitch.to_degrees() / ANGLE_LIMIT_DEG,
                throttle,
                yaw: rate_stick(HEADING_GAIN * wrap(wanted_heading - heading)),
                arm: false,
                angle: true,
            },
        }
    }
}

/// A body rate (rad/s) as a stick deflection.
fn rate_stick(rate: f64) -> f64 {
    (rate.to_degrees() / MAX_RATE_DPS).clamp(-1.0, 1.0)
}

/// An angle wrapped into `(-π, π]`.
fn wrap(angle: f64) -> f64 {
    let turn = std::f64::consts::TAU;
    let wrapped = angle.rem_euclid(turn);
    if wrapped > std::f64::consts::PI {
        wrapped - turn
    } else {
        wrapped
    }
}

#[cfg(test)]
mod tests;
