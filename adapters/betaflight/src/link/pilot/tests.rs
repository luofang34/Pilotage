#![allow(clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use super::{DEMAND_TIMEOUT, Demands, Family, Pilot};
use crate::adapter::ENVELOPE;
use crate::direct::DirectDemand;
use crate::outer_loop::{Demand, Estimate};

const DT: f64 = 0.02;

fn hovering() -> Estimate {
    Estimate {
        heading: 0.0,
        roll: 0.0,
        pitch: 0.0,
        horizontal: None,
        altitude: 3.0,
        climb_rate: 0.0,
    }
}

fn climb(now: Instant) -> Demands {
    Demands {
        family: Family::Assisted,
        velocity: Demand {
            up: 1.0,
            ..Demand::default()
        },
        velocity_at: Some(now),
        ..Demands::default()
    }
}

fn rates(now: Instant, thrust: f64) -> Demands {
    Demands {
        family: Family::Direct,
        direct: Some(DirectDemand::Rates {
            roll: 1.0,
            pitch: 0.0,
            yaw: 0.0,
            thrust,
        }),
        direct_at: Some(now),
        ..Demands::default()
    }
}

/// A pilot flying assisted at a steady throttle.
fn airborne(now: Instant) -> (Pilot, f64) {
    let mut pilot = Pilot::new(ENVELOPE);
    let mut throttle = 0.0;
    for _ in 0..50 {
        throttle = pilot
            .on_poll(&climb(now), &hovering(), true, true, now, DT)
            .throttle;
    }
    (pilot, throttle)
}

#[test]
fn disarmed_sticks_are_idle_with_the_switch_as_asked() {
    let now = Instant::now();
    let mut pilot = Pilot::new(ENVELOPE);
    let sticks = pilot.on_poll(&rates(now, 0.5), &hovering(), false, true, now, DT);
    assert_eq!(
        (sticks.throttle, sticks.arm, sticks.angle),
        (0.0, true, true)
    );
    assert!(pilot.on_demands(&rates(now, 0.5), now).is_none());
}

#[test]
fn a_direct_demand_leaves_at_once_in_acro_with_the_throttle_held_until_reached() {
    let now = Instant::now();
    let (mut pilot, throttle) = airborne(now);
    assert!(throttle > 0.3);
    let held = pilot
        .on_demands(&rates(now, 0.0), now)
        .expect("direct flies");
    assert!(!held.angle && held.roll > 0.0 && held.arm);
    assert_eq!(held.throttle, throttle, "no drop on entering direct flight");
    let taken = pilot
        .on_demands(&rates(now, throttle), now)
        .expect("direct flies");
    let after = pilot
        .on_demands(&rates(now, 0.2), now)
        .expect("direct flies");
    assert_eq!((taken.throttle, after.throttle), (throttle, 0.2));
}

#[test]
fn a_stale_direct_demand_falls_back_to_an_assisted_hold_at_the_last_throttle() {
    let now = Instant::now();
    let (mut pilot, _) = airborne(now);
    pilot.on_demands(&rates(now, 0.6), now);
    let later = now + DEMAND_TIMEOUT + Duration::from_millis(1);
    let assisted = pilot.on_poll(&rates(now, 0.6), &hovering(), true, true, later, DT);
    assert!(assisted.angle, "back in ANGLE");
    assert!(pilot.is_flying(), "still airborne");
    assert!((assisted.throttle - 0.6).abs() < 0.05, "{assisted:?}");
    assert!(pilot.on_demands(&rates(now, 0.6), later).is_none());
}

#[test]
fn an_assisted_family_never_sends_frame_by_frame() {
    let now = Instant::now();
    let (mut pilot, _) = airborne(now);
    let mut demands = rates(now, 0.5);
    demands.family = Family::Assisted;
    assert!(pilot.on_demands(&demands, now).is_none());
}
