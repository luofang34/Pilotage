//! The holding pattern flown by time and heading, for a vehicle that has
//! lost its position: the flight controller still holds a heading, so the
//! racetrack goes on as turns at a fixed rate and legs of a fixed time, the
//! way a pilot holds without a fix to cross.
//!
//! In still air such a circuit closes on itself. In a wind, each turn
//! drifts downwind for its whole length, and the legs fly through the air,
//! not over the ground. So the inbound leg is flown on the heading that
//! makes good the inbound course, and the outbound leg on the heading and
//! for the time that bring the circuit's whole movement over the ground,
//! turns included, back to nothing in the last known wind. Without a wind
//! the outbound leg is the inbound leg's reciprocal for as long.

use core::f64::consts::PI;

use super::HoldingLeg;

/// The circuit the vehicle flies by time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TimedCircuit {
    /// The inbound course, radians clockwise from true north.
    inbound_rad: f64,
    /// `1` for right turns, `-1` for left turns.
    sense: f64,
    /// The airspeed the circuit is flown at, m/s.
    airspeed_mps: f64,
    /// The rate of turn through the air, rad/s.
    turn_rate_rps: f64,
    /// The inbound leg time, seconds.
    leg_time_s: f64,
    stage: Stage,
    /// The heading commanded on the last step, radians.
    heading_rad: f64,
    last_s: f64,
}

/// The part of a timed circuit flown.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    /// The inbound leg, until a time.
    Inbound { until_s: f64 },
    /// The turn onto the outbound heading.
    TurnToOutbound,
    /// The outbound leg on its heading, until a time.
    Outbound { heading_rad: f64, until_s: f64 },
    /// The turn onto the inbound heading.
    TurnToInbound,
}

/// What a timed circuit is flown with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CircuitShape {
    /// The inbound course, radians clockwise from true north.
    pub(crate) inbound_rad: f64,
    /// Whether the turns are to the right.
    pub(crate) right_turns: bool,
    /// The airspeed, m/s.
    pub(crate) airspeed_mps: f64,
    /// The turn radius through the air, metres.
    pub(crate) turn_radius_m: f64,
    /// The inbound leg time, seconds.
    pub(crate) leg_time_s: f64,
}

impl TimedCircuit {
    /// The circuit that takes over from the geometric pattern in `leg`,
    /// `elapsed_s` into it, with its outbound leg lasting
    /// `outbound_time_s`, at `now_s`, with `heading_rad` commanded last.
    pub(crate) fn take_over(
        shape: CircuitShape,
        leg: HoldingLeg,
        elapsed_s: f64,
        outbound_time_s: f64,
        heading_rad: f64,
        now_s: f64,
        wind: [f64; 2],
    ) -> Self {
        let mut circuit = Self {
            inbound_rad: shape.inbound_rad,
            sense: if shape.right_turns { 1.0 } else { -1.0 },
            airspeed_mps: shape.airspeed_mps,
            turn_rate_rps: shape.airspeed_mps / shape.turn_radius_m,
            leg_time_s: shape.leg_time_s,
            stage: Stage::TurnToOutbound,
            heading_rad,
            last_s: now_s,
        };
        circuit.stage = match leg {
            HoldingLeg::Inbound => Stage::Inbound {
                until_s: now_s + (shape.leg_time_s - elapsed_s).max(0.0),
            },
            HoldingLeg::Outbound => Stage::Outbound {
                heading_rad: circuit.outbound(wind).0,
                until_s: now_s + (outbound_time_s - elapsed_s).max(0.0),
            },
            HoldingLeg::OutboundTurn => Stage::TurnToInbound,
            HoldingLeg::ToFix | HoldingLeg::Entry(_) | HoldingLeg::FixTurn => Stage::TurnToOutbound,
        };
        circuit
    }

    /// The true heading to command at `now_s` in `wind` (north and east,
    /// m/s).
    pub(crate) fn step(&mut self, now_s: f64, wind: [f64; 2]) -> f64 {
        let dt_s = (now_s - self.last_s).max(0.0);
        self.last_s = now_s;
        match self.stage {
            Stage::Inbound { until_s } => {
                self.heading_rad = self.inbound_heading(wind);
                if now_s >= until_s {
                    self.stage = Stage::TurnToOutbound;
                }
            }
            Stage::Outbound {
                heading_rad,
                until_s,
            } => {
                self.heading_rad = heading_rad;
                if now_s >= until_s {
                    self.stage = Stage::TurnToInbound;
                }
            }
            Stage::TurnToOutbound => {
                let (heading_rad, time_s) = self.outbound(wind);
                if self.turn_toward(heading_rad, dt_s) {
                    self.stage = Stage::Outbound {
                        heading_rad,
                        until_s: now_s + time_s,
                    };
                }
            }
            Stage::TurnToInbound => {
                if self.turn_toward(self.inbound_heading(wind), dt_s) {
                    self.stage = Stage::Inbound {
                        until_s: now_s + self.leg_time_s,
                    };
                }
            }
        }
        self.heading_rad
    }

    /// Whether the inbound leg is flown now.
    #[cfg(test)]
    pub(crate) const fn on_inbound(&self) -> bool {
        matches!(self.stage, Stage::Inbound { .. })
    }

    /// Moves the commanded heading `dt_s` worth of turn toward `target`
    /// in the pattern's direction; true once it is there.
    fn turn_toward(&mut self, target: f64, dt_s: f64) -> bool {
        let to_go = (self.sense * (target - self.heading_rad)).rem_euclid(2.0 * PI);
        let step = self.turn_rate_rps * dt_s;
        if to_go <= step {
            self.heading_rad = target;
            true
        } else {
            self.heading_rad = (self.heading_rad + self.sense * step).rem_euclid(2.0 * PI);
            false
        }
    }

    /// The heading that makes good the inbound course in `wind`.
    fn inbound_heading(&self, wind: [f64; 2]) -> f64 {
        let across = cross(unit(self.inbound_rad), wind);
        let ratio = (across / self.airspeed_mps).clamp(-1.0, 1.0);
        self.inbound_rad - ratio.asin()
    }

    /// The outbound heading and time that close the circuit in `wind`: the
    /// inbound leg's movement over the ground, and two half turns' drift,
    /// taken back on the outbound leg.
    fn outbound(&self, wind: [f64; 2]) -> (f64, f64) {
        let reciprocal = (self.inbound_rad + PI, self.leg_time_s);
        let inbound = unit(self.inbound_heading(wind));
        let turns_s = 2.0 * PI / self.turn_rate_rps;
        let leg = self.leg_time_s;
        // The circuit's movement before the outbound leg, negated.
        let back = [
            -(leg * (self.airspeed_mps * inbound[0] + wind[0]) + turns_s * wind[0]),
            -(leg * (self.airspeed_mps * inbound[1] + wind[1]) + turns_s * wind[1]),
        ];
        // The time `t` with |back / t - wind| = airspeed.
        let speed2 = self.airspeed_mps * self.airspeed_mps;
        let a = speed2 - dot(wind, wind);
        let b = dot(back, wind);
        let c = dot(back, back);
        if a <= 0.0 {
            return reciprocal;
        }
        let time_s = (-b + (b * b + a * c).sqrt()) / a;
        if !time_s.is_finite() || time_s <= 0.0 {
            return reciprocal;
        }
        let air = [back[0] / time_s - wind[0], back[1] / time_s - wind[1]];
        (air[1].atan2(air[0]), time_s)
    }
}

fn unit(course: f64) -> [f64; 2] {
    [course.cos(), course.sin()]
}

/// The component of `vector` to the right of the unit `direction`.
fn cross(direction: [f64; 2], vector: [f64; 2]) -> f64 {
    -direction[1] * vector[0] + direction[0] * vector[1]
}

fn dot(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

#[cfg(test)]
mod tests;
