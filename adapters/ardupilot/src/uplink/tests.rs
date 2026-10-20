use std::time::Instant;

use tokio::sync::{mpsc, watch};

use pilotage_mavlink::codec::{MAV_CMD_COMPONENT_ARM_DISARM, MAV_CMD_DO_SET_MODE};
use pilotage_mavlink::{BodyVelocityDemand, OffboardSetpoint, OutboundCommand};

use super::{ArduPilotUplink, DirectOutcome, DisarmOutcome, VelocityOutcome};

/// The body velocity a setpoint lane holds, if it holds one.
fn body_velocity(setpoint: Option<OffboardSetpoint>) -> Option<BodyVelocityDemand> {
    match setpoint? {
        OffboardSetpoint::BodyVelocity(demand) => Some(demand),
        OffboardSetpoint::AttitudeThrust(_) => None,
    }
}

pub(super) struct Lanes {
    pub(super) commands: mpsc::Receiver<OutboundCommand>,
    pub(super) velocity: watch::Receiver<Option<OffboardSetpoint>>,
}

pub(super) fn uplink() -> (ArduPilotUplink, Lanes) {
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

/// Enters manually commanded direct flight, then returns to velocity control.
fn airborne(uplink: &mut ArduPilotUplink, lanes: &mut Lanes, now: Instant) {
    uplink.arm(false, now);
    assert_eq!(
        uplink.apply_direct(
            pilotage_mavlink::codec::AttitudeTargetKind::BodyRates([0.0; 3]),
            0.5,
            true,
            0.4,
            now,
        ),
        DirectOutcome::Streamed { slewed: false }
    );
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
        uplink.apply_velocity(FORWARD, false, false),
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
        uplink.apply_velocity(FORWARD, false, true),
        VelocityOutcome::HeldOnGround
    );
    assert!(drain(&mut lanes).is_empty());
    assert!(!lanes.velocity.has_changed().expect("lane open"));
}

#[test]
fn a_climb_intent_on_the_ground_does_not_take_off() {
    let (mut uplink, mut lanes) = uplink();
    assert_eq!(
        uplink.apply_velocity(CLIMB, false, true),
        VelocityOutcome::HeldOnGround
    );
    assert!(drain(&mut lanes).is_empty());
    assert!(!lanes.velocity.has_changed().expect("lane open"));
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
        uplink.apply_velocity(FORWARD, false, true),
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
        uplink.apply_velocity(FORWARD, false, true),
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
