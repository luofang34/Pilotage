#![allow(clippy::expect_used, clippy::panic)]

use std::time::{Duration, Instant};

use tokio::sync::{mpsc, watch};

use pilotage_mavlink::OffboardSetpoint;
use pilotage_mavlink::codec::AttitudeTargetKind;

use super::{DirectOutcome, fc_thrust};
use crate::uplink::{ArduPilotUplink, VelocityOutcome};

const HOVER: f32 = 0.39;
const RATES: AttitudeTargetKind = AttitudeTargetKind::BodyRates([0.0, 0.0, 0.5]);

fn uplink() -> (ArduPilotUplink, watch::Receiver<Option<OffboardSetpoint>>) {
    let (command_tx, _commands) = mpsc::channel(16);
    let (setpoint_tx, setpoints) = watch::channel(None);
    (
        ArduPilotUplink::new(command_tx, setpoint_tx, 1, 1),
        setpoints,
    )
}

fn thrust_of(setpoint: Option<OffboardSetpoint>) -> f32 {
    match setpoint {
        Some(OffboardSetpoint::AttitudeThrust(demand)) => demand.thrust,
        other => panic!("an attitude-thrust setpoint, got {other:?}"),
    }
}

#[test]
fn hover_sits_at_the_canonical_midpoint() {
    assert_eq!(fc_thrust(0.5, HOVER), HOVER);
    assert_eq!(fc_thrust(0.0, HOVER), 0.0);
    assert_eq!(fc_thrust(1.0, HOVER), 1.0);
    assert!((fc_thrust(0.75, HOVER) - (HOVER + 0.5 * (1.0 - HOVER))).abs() < 1e-6);
    assert_eq!(fc_thrust(f32::NAN, HOVER), HOVER);
}

#[test]
fn a_switch_into_direct_starts_from_hover_and_slews() {
    let (mut uplink, mut setpoints) = uplink();
    let now = Instant::now();
    // Flying under velocity control, then the operator switches with the
    // thrust stick at the top.
    let streamed = uplink.apply_direct(RATES, 0.5, true, HOVER, now);
    assert_eq!(streamed, DirectOutcome::Streamed { slewed: false });
    assert_eq!(
        uplink.apply_velocity([0.0; 4], false, true, -3.0, now),
        VelocityOutcome::Streamed { constrained: false }
    );
    setpoints.borrow_and_update();

    let first = uplink.apply_direct(RATES, 1.0, true, HOVER, now);
    assert_eq!(first, DirectOutcome::Streamed { slewed: true });
    let thrust = thrust_of(*setpoints.borrow_and_update());
    assert!(
        (thrust - HOVER).abs() <= 1.5 / 60.0 + 1e-6,
        "first direct thrust {thrust} stays within one slew step of hover"
    );

    // A second later the slew has reached full thrust.
    let later = now + Duration::from_millis(100);
    for step in 0..10 {
        uplink.apply_direct(
            RATES,
            1.0,
            true,
            HOVER,
            later + Duration::from_millis(100 * step),
        );
    }
    assert_eq!(thrust_of(*setpoints.borrow_and_update()), 1.0);
}

#[test]
fn direct_flight_waits_for_arm_and_yields_to_landing() {
    let (mut uplink, _setpoints) = uplink();
    let now = Instant::now();
    assert_eq!(
        uplink.apply_direct(RATES, 0.5, false, HOVER, now),
        DirectOutcome::NotArmed
    );
    uplink.apply_direct(RATES, 0.5, true, HOVER, now);
    uplink.disarm(now);
    assert_eq!(
        uplink.apply_direct(RATES, 0.5, true, HOVER, now),
        DirectOutcome::Landing
    );
}
