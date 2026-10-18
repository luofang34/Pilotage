#![allow(clippy::expect_used, clippy::panic)]

use super::{Demand, Envelope, Estimate, Horizontal, OuterLoop};
use crate::rc::{ANGLE_LIMIT_DEG, MAX_RATE_DPS, Sticks};

const ENVELOPE: Envelope = Envelope {
    horizontal: 3.0,
    vertical: 1.5,
    yaw_rate: 1.5,
};
const DT: f64 = 0.02;
const GRAVITY: f64 = 9.806_65;

/// A point-mass stand-in for Betaflight's ANGLE mode and the airframe:
/// tilt follows the sticks with a 0.1 s lag, thrust is proportional to
/// throttle with hover at `hover`, the ground stops a fall.
struct Plant {
    hover: f64,
    /// Vertical drag, (m/s²)/(m/s): a climb then needs more than hover.
    vertical_drag: f64,
    north: f64,
    east: f64,
    up: f64,
    velocity: [f64; 3],
    heading: f64,
    roll: f64,
    pitch: f64,
}

impl Plant {
    fn landed(hover: f64, heading: f64) -> Self {
        Self {
            hover,
            vertical_drag: 0.0,
            north: 0.0,
            east: 0.0,
            up: 0.0,
            velocity: [0.0; 3],
            heading,
            roll: 0.0,
            pitch: 0.0,
        }
    }

    fn estimate(&self) -> Estimate {
        Estimate {
            heading: self.heading,
            roll: self.roll,
            pitch: self.pitch,
            horizontal: Some(Horizontal {
                position: [self.north, self.east],
                velocity: [self.velocity[0], self.velocity[1]],
            }),
            altitude: self.up,
            climb_rate: self.velocity[2],
        }
    }

    fn step(&mut self, sticks: Sticks) {
        let lag = DT / 0.1;
        let roll_target = (sticks.roll * ANGLE_LIMIT_DEG).to_radians();
        let nose_down_target = (sticks.pitch * ANGLE_LIMIT_DEG).to_radians();
        self.roll += (roll_target - self.roll) * lag;
        self.pitch += (-nose_down_target - self.pitch) * lag;
        self.heading += (sticks.yaw * MAX_RATE_DPS).to_radians() * DT;
        let thrust = GRAVITY * sticks.throttle / self.hover;
        let forward = thrust * (-self.pitch).sin();
        let right = thrust * self.roll.sin();
        let (sin, cos) = self.heading.sin_cos();
        let accel = [
            forward * cos - right * sin - 0.3 * self.velocity[0],
            forward * sin + right * cos - 0.3 * self.velocity[1],
            thrust * self.roll.cos() * self.pitch.cos()
                - GRAVITY
                - self.vertical_drag * self.velocity[2],
        ];
        for (velocity, accel) in self.velocity.iter_mut().zip(accel) {
            *velocity += accel * DT;
        }
        self.north += self.velocity[0] * DT;
        self.east += self.velocity[1] * DT;
        self.up += self.velocity[2] * DT;
        if self.up <= 0.0 {
            self.up = 0.0;
            self.velocity = [0.0; 3];
        }
    }
}

fn fly(outer: &mut OuterLoop, plant: &mut Plant, demand: Demand, seconds: f64) {
    for _ in 0..(seconds / DT).round() as u32 {
        let sticks = outer.update(demand, &plant.estimate(), DT);
        plant.step(sticks);
    }
}

fn climb(up: f64) -> Demand {
    Demand {
        up,
        ..Demand::default()
    }
}

#[test]
fn a_landed_quad_idles_until_asked_to_climb() {
    let mut outer = OuterLoop::new(ENVELOPE);
    let mut plant = Plant::landed(0.55, 0.0);
    fly(&mut outer, &mut plant, Demand::default(), 2.0);
    assert!(!outer.is_flying());
    assert_eq!(
        outer.update(Demand::default(), &plant.estimate(), DT),
        Sticks::IDLE
    );
    assert_eq!(plant.up, 0.0);
}

#[test]
fn a_climb_demand_takes_off_and_neutral_then_holds_altitude() {
    let mut outer = OuterLoop::new(ENVELOPE);
    let mut plant = Plant::landed(0.6, 0.0);
    fly(&mut outer, &mut plant, climb(1.0), 6.0);
    assert!(outer.is_flying());
    assert!(plant.up > 2.5, "climbed to {}", plant.up);
    assert!(
        (plant.velocity[2] - 1.0).abs() < 0.2,
        "climb rate {}",
        plant.velocity[2]
    );
    // The hover integrator learns the airframe's 0.6 from its 0.5 guess.
    fly(&mut outer, &mut plant, Demand::default(), 10.0);
    let held = plant.up;
    fly(&mut outer, &mut plant, Demand::default(), 5.0);
    assert!(
        (plant.up - held).abs() < 0.2,
        "altitude {held} -> {}",
        plant.up
    );
    assert!(
        (outer.hover_throttle() - 0.6).abs() < 0.05,
        "learned {}",
        outer.hover_throttle()
    );
}

#[test]
fn forward_follows_the_heading_and_neutral_holds_position() {
    let mut outer = OuterLoop::new(ENVELOPE);
    // Heading east: forward is east.
    let mut plant = Plant::landed(0.55, std::f64::consts::FRAC_PI_2);
    fly(&mut outer, &mut plant, climb(1.0), 3.0);
    fly(&mut outer, &mut plant, Demand::default(), 2.0);
    let forward = Demand {
        forward: 2.0,
        ..Demand::default()
    };
    fly(&mut outer, &mut plant, forward, 6.0);
    assert!(
        (plant.velocity[1] - 2.0).abs() < 0.2,
        "east speed {}",
        plant.velocity[1]
    );
    assert!(
        plant.velocity[0].abs() < 0.2,
        "north speed {}",
        plant.velocity[0]
    );
    fly(&mut outer, &mut plant, Demand::default(), 4.0);
    let (north, east) = (plant.north, plant.east);
    fly(&mut outer, &mut plant, Demand::default(), 5.0);
    let drift = (plant.north - north).hypot(plant.east - east);
    assert!(drift < 0.5, "drifted {drift} m while holding");
}

#[test]
fn a_right_demand_rolls_right_and_a_forward_one_pitches_nose_down() {
    let mut outer = OuterLoop::new(ENVELOPE);
    let mut plant = Plant::landed(0.55, 0.0);
    fly(&mut outer, &mut plant, climb(1.0), 2.0);
    let estimate = plant.estimate();
    let right = outer.update(
        Demand {
            right: 3.0,
            ..Demand::default()
        },
        &estimate,
        DT,
    );
    assert!(right.roll > 0.3 && right.pitch.abs() < 0.05, "{right:?}");
    let forward = outer.update(
        Demand {
            forward: 3.0,
            ..Demand::default()
        },
        &estimate,
        DT,
    );
    assert!(forward.pitch > 0.3, "{forward:?}");
}

#[test]
fn yaw_rate_maps_to_the_yaw_stick() {
    let mut outer = OuterLoop::new(ENVELOPE);
    let mut plant = Plant::landed(0.55, 0.0);
    fly(&mut outer, &mut plant, climb(1.0), 2.0);
    let sticks = outer.update(
        Demand {
            yaw_rate: 1.0,
            ..Demand::default()
        },
        &plant.estimate(),
        DT,
    );
    assert!((sticks.yaw - 1f64.to_degrees() / MAX_RATE_DPS).abs() < 1e-9);
}

#[test]
fn a_descent_to_the_ground_lands_and_cuts_throttle() {
    let mut outer = OuterLoop::new(ENVELOPE);
    let mut plant = Plant::landed(0.55, 0.0);
    fly(&mut outer, &mut plant, climb(1.0), 2.0);
    fly(&mut outer, &mut plant, climb(-1.0), 6.0);
    assert!(!outer.is_flying());
    assert_eq!(plant.up, 0.0);
    assert_eq!(
        outer.update(climb(-1.0), &plant.estimate(), DT).throttle,
        0.0
    );
}

#[test]
fn tilt_never_exceeds_the_limit() {
    let mut outer = OuterLoop::new(ENVELOPE);
    let mut plant = Plant::landed(0.55, 0.0);
    fly(&mut outer, &mut plant, climb(1.0), 2.0);
    let mut estimate = plant.estimate();
    if let Some(horizontal) = estimate.horizontal.as_mut() {
        horizontal.velocity = [-20.0, 20.0];
    }
    let sticks = outer.update(
        Demand {
            forward: 3.0,
            right: -3.0,
            ..Demand::default()
        },
        &estimate,
        DT,
    );
    assert!(sticks.roll.abs() * ANGLE_LIMIT_DEG <= 25.0 + 1e-9);
    assert!(sticks.pitch.abs() * ANGLE_LIMIT_DEG <= 25.0 + 1e-9);
}

#[test]
fn resuming_in_flight_keeps_the_throttle_and_holds_altitude() {
    let mut outer = OuterLoop::new(ENVELOPE);
    let mut plant = Plant::landed(0.6, 0.0);
    plant.up = 5.0;
    outer.resume(0.6, true);
    assert!(outer.is_flying());
    let first = outer.update(Demand::default(), &plant.estimate(), DT);
    assert!((first.throttle - 0.6).abs() < 0.01, "{first:?}");
    fly(&mut outer, &mut plant, Demand::default(), 5.0);
    assert!((plant.up - 5.0).abs() < 0.3, "altitude {}", plant.up);
}

#[test]
fn the_hover_throttle_is_learned_in_a_hover_not_in_a_climb() {
    let mut outer = OuterLoop::new(ENVELOPE);
    let mut plant = Plant::landed(0.55, 0.0);
    plant.vertical_drag = 0.6;
    // Before any settled hover the integrator stands in.
    fly(&mut outer, &mut plant, climb(1.5), 2.0);
    assert_eq!(outer.settled_hover, None);
    assert_eq!(outer.hover_throttle(), outer.hover);
    // A settled hover teaches the hover throttle.
    fly(&mut outer, &mut plant, Demand::default(), 15.0);
    let hover = outer.hover_throttle();
    assert!((hover - 0.55).abs() < 0.01, "learned {hover}");
    // A climb against drag drives the integrator above hover; the hover
    // throttle stays what the hover taught.
    fly(&mut outer, &mut plant, climb(1.5), 5.0);
    assert!(
        outer.hover > 0.56,
        "the climb integrator reads {}",
        outer.hover
    );
    assert!(
        (outer.hover_throttle() - hover).abs() < 1e-9,
        "{} moved off the hover {hover}",
        outer.hover_throttle()
    );
}
