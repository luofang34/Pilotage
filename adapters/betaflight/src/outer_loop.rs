//! The host-side outer loop that gives a Betaflight quad velocity
//! semantics.
//!
//! Betaflight has no velocity mode an MSP client can command: in ANGLE
//! mode the sticks set a tilt angle, the throttle stick sets collective
//! thrust, and the yaw stick sets a yaw rate. This loop closes velocity
//! around those inputs from Betaflight's own estimate (GNSS ground
//! velocity, fused altitude and climb rate, attitude):
//!
//! - horizontal: body velocity demand → NED velocity error → acceleration
//!   (PI) → tilt angles → roll and pitch sticks. A demand inside the
//!   deadband holds position: the loop brakes, captures the position once
//!   slow, and steers back to it.
//! - vertical: climb demand → climb-rate PI around a learned hover
//!   throttle, with tilt compensation. A neutral demand holds altitude.
//! - yaw: the demanded rate maps straight to the yaw stick.
//!
//! On the ground the throttle stays at idle until the demand asks to
//! climb, so take-off is the operator's climb command.

use crate::rc::{ANGLE_LIMIT_DEG, Sticks, YAW_RATE_LIMIT_DPS};

/// Standard gravity, m/s².
const GRAVITY: f64 = 9.806_65;
/// Demands below this fraction of the advertised envelope are neutral.
pub(crate) const NEUTRAL_FRACTION: f64 = 0.05;
/// Position-hold gain, (m/s)/m.
const POSITION_GAIN: f64 = 0.8;
/// Fastest the position hold steers back, m/s.
const HOLD_SPEED_LIMIT: f64 = 1.0;
/// Below this ground speed a braking quad captures its hold position.
const CAPTURE_SPEED: f64 = 0.3;
/// Velocity-loop proportional gain, (m/s²)/(m/s).
const VELOCITY_GAIN: f64 = 1.8;
/// Velocity-loop integral gain, (m/s²)/m.
const VELOCITY_INTEGRAL_GAIN: f64 = 0.4;
/// Largest integral acceleration, m/s².
const VELOCITY_INTEGRAL_LIMIT: f64 = 1.5;
/// Largest commanded tilt, degrees.
const TILT_LIMIT_DEG: f64 = 25.0;
/// Altitude-hold gain, (m/s)/m.
const ALTITUDE_GAIN: f64 = 0.5;
/// Fastest the altitude hold climbs or sinks back, m/s.
const ALTITUDE_SPEED_LIMIT: f64 = 1.0;
/// Climb-rate proportional gain, throttle per m/s.
const CLIMB_GAIN: f64 = 0.05;
/// Climb-rate integral gain, throttle per metre.
const CLIMB_INTEGRAL_GAIN: f64 = 0.05;
/// The hover throttle the integrator starts from before it has learned.
const HOVER_GUESS: f64 = 0.5;
/// Bounds on the learned hover throttle.
const HOVER_RANGE: (f64, f64) = (0.2, 0.8);
/// Climb demand that lifts a landed quad off, m/s.
const TAKEOFF_CLIMB: f64 = 0.15;
/// Below this altitude a sinking, settled quad counts as landed, m.
const LANDED_ALTITUDE: f64 = 0.3;
/// How long a quad must sit settled near the ground to count as landed.
const LANDED_DWELL_S: f64 = 1.0;

/// A velocity demand in the body frame, already inside the envelope.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Demand {
    /// Forward, m/s.
    pub(crate) forward: f64,
    /// Right, m/s.
    pub(crate) right: f64,
    /// Up, m/s.
    pub(crate) up: f64,
    /// Yaw rate, rad/s, positive clockwise.
    pub(crate) yaw_rate: f64,
}

/// What the loop knows of the vehicle this step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Estimate {
    /// Heading, radians clockwise from north.
    pub(crate) heading: f64,
    /// Roll and pitch, radians (for tilt compensation).
    pub(crate) roll: f64,
    /// Pitch, radians.
    pub(crate) pitch: f64,
    /// Local north/east position and velocity, when a fix is usable.
    pub(crate) horizontal: Option<Horizontal>,
    /// Altitude, m.
    pub(crate) altitude: f64,
    /// Climb rate, m/s.
    pub(crate) climb_rate: f64,
}

/// The horizontal part of an [`Estimate`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Horizontal {
    /// North, east, m.
    pub(crate) position: [f64; 2],
    /// North, east, m/s.
    pub(crate) velocity: [f64; 2],
}

/// The envelope a full demand corresponds to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Envelope {
    /// Horizontal speed, m/s.
    pub(crate) horizontal: f64,
    /// Vertical speed, m/s.
    pub(crate) vertical: f64,
    /// Yaw rate, rad/s.
    pub(crate) yaw_rate: f64,
}

/// The loop's memory between steps.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct OuterLoop {
    envelope: Envelope,
    flying: bool,
    settled_s: f64,
    hold_position: Option<[f64; 2]>,
    hold_altitude: Option<f64>,
    velocity_integral: [f64; 2],
    hover: f64,
}

impl OuterLoop {
    /// A loop for a landed quad.
    pub(crate) fn new(envelope: Envelope) -> Self {
        Self {
            envelope,
            flying: false,
            settled_s: 0.0,
            hold_position: None,
            hold_altitude: None,
            velocity_integral: [0.0; 2],
            hover: HOVER_GUESS,
        }
    }

    /// Whether the loop considers the quad airborne.
    pub(crate) fn is_flying(&self) -> bool {
        self.flying
    }

    /// The learned hover throttle.
    pub(crate) fn hover_throttle(&self) -> f64 {
        self.hover
    }

    /// Forgets the flight: the quad was disarmed or the link restarted.
    /// The learned hover throttle stays, since the airframe did not change.
    pub(crate) fn reset(&mut self) {
        *self = Self {
            hover: self.hover,
            ..Self::new(self.envelope)
        };
    }

    /// One step: the stick positions (arm switch left off; the caller
    /// owns it) for `demand` given `estimate`, `dt` seconds after the last.
    pub(crate) fn update(&mut self, demand: Demand, estimate: &Estimate, dt: f64) -> Sticks {
        let dt = dt.clamp(0.0, 0.1);
        self.update_phase(demand, estimate, dt);
        let yaw = (demand.yaw_rate.to_degrees() / YAW_RATE_LIMIT_DPS).clamp(-1.0, 1.0);
        if !self.flying {
            return Sticks {
                yaw: 0.0,
                ..Sticks::IDLE
            };
        }
        let (forward_accel, right_accel) = self.horizontal(demand, estimate, dt);
        let nose_down = (forward_accel / GRAVITY).atan().to_degrees();
        let roll_right = (right_accel / GRAVITY).atan().to_degrees();
        let throttle = self.vertical(demand, estimate, dt);
        Sticks {
            roll: (roll_right.clamp(-TILT_LIMIT_DEG, TILT_LIMIT_DEG)) / ANGLE_LIMIT_DEG,
            pitch: (nose_down.clamp(-TILT_LIMIT_DEG, TILT_LIMIT_DEG)) / ANGLE_LIMIT_DEG,
            throttle,
            yaw,
            arm: false,
        }
    }

    fn update_phase(&mut self, demand: Demand, estimate: &Estimate, dt: f64) {
        if !self.flying {
            if demand.up >= TAKEOFF_CLIMB {
                self.flying = true;
                self.settled_s = 0.0;
                self.hold_altitude = None;
                self.hold_position = None;
                self.velocity_integral = [0.0; 2];
            }
            return;
        }
        let settled = estimate.altitude < LANDED_ALTITUDE
            && estimate.climb_rate.abs() < 0.1
            && demand.up <= 0.0
            && self.hold_altitude.is_none_or(|hold| hold < LANDED_ALTITUDE);
        self.settled_s = if settled { self.settled_s + dt } else { 0.0 };
        if self.settled_s >= LANDED_DWELL_S && demand.up < 0.0 {
            self.flying = false;
        }
    }

    /// Forward and right acceleration demands, m/s².
    fn horizontal(&mut self, demand: Demand, estimate: &Estimate, dt: f64) -> (f64, f64) {
        let (sin, cos) = estimate.heading.sin_cos();
        let to_body =
            |[north, east]: [f64; 2]| (north * cos + east * sin, -north * sin + east * cos);
        let Some(horizontal) = estimate.horizontal else {
            // No usable fix: fly the demand open loop as a proportional tilt
            // so the operator keeps control, and hold nothing.
            self.hold_position = None;
            self.velocity_integral = [0.0; 2];
            return (
                demand.forward * VELOCITY_GAIN * 0.5,
                demand.right * VELOCITY_GAIN * 0.5,
            );
        };
        let neutral =
            demand.forward.hypot(demand.right) < self.envelope.horizontal * NEUTRAL_FRACTION;
        let target = if neutral {
            self.hold_velocity(&horizontal)
        } else {
            self.hold_position = None;
            [
                demand.forward * cos - demand.right * sin,
                demand.forward * sin + demand.right * cos,
            ]
        };
        let mut accel = [0.0; 2];
        for axis in 0..2 {
            let error = target[axis] - horizontal.velocity[axis];
            self.velocity_integral[axis] = (self.velocity_integral[axis]
                + VELOCITY_INTEGRAL_GAIN * error * dt)
                .clamp(-VELOCITY_INTEGRAL_LIMIT, VELOCITY_INTEGRAL_LIMIT);
            accel[axis] = VELOCITY_GAIN * error + self.velocity_integral[axis];
        }
        to_body(accel)
    }

    /// The NED velocity that holds position: zero while braking, then a
    /// pull back to the position captured once slow.
    fn hold_velocity(&mut self, horizontal: &Horizontal) -> [f64; 2] {
        let speed = horizontal.velocity[0].hypot(horizontal.velocity[1]);
        if self.hold_position.is_none() && speed < CAPTURE_SPEED {
            self.hold_position = Some(horizontal.position);
        }
        let Some(hold) = self.hold_position else {
            return [0.0; 2];
        };
        let pull = [
            POSITION_GAIN * (hold[0] - horizontal.position[0]),
            POSITION_GAIN * (hold[1] - horizontal.position[1]),
        ];
        let magnitude = pull[0].hypot(pull[1]);
        if magnitude > HOLD_SPEED_LIMIT {
            let scale = HOLD_SPEED_LIMIT / magnitude;
            [pull[0] * scale, pull[1] * scale]
        } else {
            pull
        }
    }

    /// The throttle, 0..1.
    fn vertical(&mut self, demand: Demand, estimate: &Estimate, dt: f64) -> f64 {
        let neutral = demand.up.abs() < self.envelope.vertical * NEUTRAL_FRACTION;
        let target_climb = if neutral {
            let hold = *self.hold_altitude.get_or_insert(estimate.altitude);
            (ALTITUDE_GAIN * (hold - estimate.altitude))
                .clamp(-ALTITUDE_SPEED_LIMIT, ALTITUDE_SPEED_LIMIT)
        } else {
            self.hold_altitude = None;
            demand.up
        };
        let error = target_climb - estimate.climb_rate;
        self.hover =
            (self.hover + CLIMB_INTEGRAL_GAIN * error * dt).clamp(HOVER_RANGE.0, HOVER_RANGE.1);
        let tilt = (estimate.roll.cos() * estimate.pitch.cos()).clamp(0.7, 1.0);
        ((self.hover + CLIMB_GAIN * error) / tilt).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests;
