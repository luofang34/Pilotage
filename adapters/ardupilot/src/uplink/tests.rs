#![allow(clippy::expect_used)]

use std::time::{Duration, Instant};

use tokio::sync::{mpsc, watch};

use pilotage_mavlink::codec::{
    MAV_CMD_COMPONENT_ARM_DISARM, MAV_CMD_DO_SET_MODE, MAV_CMD_NAV_TAKEOFF,
};
use pilotage_mavlink::{BodyVelocityDemand, OffboardSetpoint, OutboundCommand};

use super::{ArduPilotUplink, DisarmOutcome, TAKEOFF_ALTITUDE_M, VelocityOutcome};

/// The body velocity a setpoint lane holds, if it holds one.
fn body_velocity(setpoint: Option<OffboardSetpoint>) -> Option<BodyVelocityDemand> {
    match setpoint? {
        OffboardSetpoint::BodyVelocity(demand) => Some(demand),
        OffboardSetpoint::AttitudeThrust(_) => None,
    }
}

struct Lanes {
    commands: mpsc::Receiver<OutboundCommand>,
    velocity: watch::Receiver<Option<OffboardSetpoint>>,
}

fn uplink() -> (ArduPilotUplink, Lanes) {
    let (command_tx, commands) = mpsc::channel(16);
    let (velocity_tx, velocity) = watch::channel(None);
    (
        ArduPilotUplink::new(command_tx, velocity_tx, 1, 1),
        Lanes { commands, velocity },
    )
}

fn drain(lanes: &mut Lanes) -> Vec<(u16, [f32; 7])> {
    let mut sent = Vec::new();
    while let Ok(command) = lanes.commands.try_recv() {
        sent.push((command.command, command.params));
    }
    sent
}

const CLIMB: [f32; 4] = [0.0, 0.0, -1.0, 0.0];
const FORWARD: [f32; 4] = [2.0, 0.0, 0.0, 0.0];

/// Arms, then climbs through a takeoff to its altitude.
fn airborne(uplink: &mut ArduPilotUplink, lanes: &mut Lanes, now: Instant) {
    uplink.arm(false, now);
    assert_eq!(
        uplink.apply_velocity(CLIMB, false, true, 0.0, now),
        VelocityOutcome::TakeoffCommanded
    );
    let done = uplink.apply_velocity(CLIMB, false, true, -TAKEOFF_ALTITUDE_M, now);
    assert_eq!(done, VelocityOutcome::Streamed { constrained: false });
    drain(lanes);
}

#[test]
fn arm_selects_guided_before_arming() {
    let (mut uplink, mut lanes) = uplink();
    uplink.arm(false, Instant::now());
    let sent = drain(&mut lanes);
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].0, MAV_CMD_DO_SET_MODE);
    assert_eq!(sent[0].1[1], 4.0, "GUIDED is ArduCopter mode 4");
    assert_eq!(
        sent[1],
        (
            MAV_CMD_COMPONENT_ARM_DISARM,
            [1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
        )
    );
}

#[test]
fn a_disarmed_vehicle_streams_nothing() {
    let (mut uplink, mut lanes) = uplink();
    assert_eq!(
        uplink.apply_velocity(FORWARD, false, false, 0.0, Instant::now()),
        VelocityOutcome::NotArmed
    );
    assert!(drain(&mut lanes).is_empty());
    assert!(!lanes.velocity.has_changed().expect("lane open"));
}

#[test]
fn landed_without_a_climb_intent_holds() {
    let (mut uplink, mut lanes) = uplink();
    let now = Instant::now();
    uplink.arm(false, now);
    drain(&mut lanes);
    assert_eq!(
        uplink.apply_velocity(FORWARD, false, true, 0.0, now),
        VelocityOutcome::HeldOnGround
    );
    assert!(drain(&mut lanes).is_empty());
    assert!(!lanes.velocity.has_changed().expect("lane open"));
}

#[test]
fn a_climb_intent_on_the_ground_takes_off_and_setpoints_wait_for_it() {
    let (mut uplink, mut lanes) = uplink();
    let now = Instant::now();
    uplink.arm(false, now);
    drain(&mut lanes);
    assert_eq!(
        uplink.apply_velocity(CLIMB, false, true, 0.0, now),
        VelocityOutcome::TakeoffCommanded
    );
    let sent = drain(&mut lanes);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, MAV_CMD_NAV_TAKEOFF);
    assert_eq!(sent[0].1[6], TAKEOFF_ALTITUDE_M);

    // Halfway up: a setpoint now would abort the takeoff.
    assert_eq!(
        uplink.apply_velocity(FORWARD, false, true, -1.0, now),
        VelocityOutcome::TakeoffInProgress
    );
    assert!(!lanes.velocity.has_changed().expect("lane open"));

    assert_eq!(
        uplink.apply_velocity(FORWARD, false, true, -2.1, now),
        VelocityOutcome::Streamed { constrained: false }
    );
    let demand = body_velocity(*lanes.velocity.borrow_and_update()).expect("a setpoint");
    assert_eq!(demand.vel_frd_mps, [2.0, 0.0, 0.0]);
}

#[test]
fn a_stalled_takeoff_hands_over_after_its_timeout() {
    let (mut uplink, mut lanes) = uplink();
    let now = Instant::now();
    uplink.arm(false, now);
    uplink.apply_velocity(CLIMB, false, true, 0.0, now);
    drain(&mut lanes);
    let later = now + Duration::from_secs(11);
    assert_eq!(
        uplink.apply_velocity(FORWARD, false, true, 0.0, later),
        VelocityOutcome::Streamed { constrained: false }
    );
}

#[test]
fn disarm_in_flight_lands_and_on_the_ground_disarms() {
    let (mut uplink, mut lanes) = uplink();
    let now = Instant::now();
    airborne(&mut uplink, &mut lanes, now);
    assert_eq!(uplink.disarm(now), DisarmOutcome::LandCommanded);
    let sent = drain(&mut lanes);
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, MAV_CMD_DO_SET_MODE);
    assert_eq!(sent[0].1[1], 9.0, "LAND is ArduCopter mode 9");
    assert_eq!(
        uplink.apply_velocity(FORWARD, false, true, -2.0, now),
        VelocityOutcome::Landing
    );

    // The FC disarms after landing; the next disarm is a real disarm.
    uplink.observe_armed(false);
    assert_eq!(uplink.disarm(now), DisarmOutcome::Disarmed);
    assert_eq!(
        drain(&mut lanes),
        vec![(MAV_CMD_COMPONENT_ARM_DISARM, [0.0; 7])]
    );
}

#[test]
fn arm_while_landing_returns_to_velocity_control() {
    let (mut uplink, mut lanes) = uplink();
    let now = Instant::now();
    airborne(&mut uplink, &mut lanes, now);
    uplink.disarm(now);
    uplink.arm(true, now);
    assert_eq!(
        uplink.apply_velocity(FORWARD, false, true, -2.0, now),
        VelocityOutcome::Streamed { constrained: false }
    );
}

#[test]
fn neutralize_sends_one_zero_setpoint_only_when_flying() {
    let (mut uplink, mut lanes) = uplink();
    let now = Instant::now();
    uplink.neutralize();
    assert!(!lanes.velocity.has_changed().expect("lane open"));

    airborne(&mut uplink, &mut lanes, now);
    lanes.velocity.borrow_and_update();
    uplink.neutralize();
    let demand = body_velocity(*lanes.velocity.borrow_and_update()).expect("a setpoint");
    assert_eq!(demand.vel_frd_mps, [0.0; 3]);
    assert_eq!(demand.yaw_rate_rps, 0.0);
}
