use std::time::Duration;

use pilotage_control_feel::{FlightFeelProfile, ValidatedFlightFeelProfile};
use pilotage_mavlink::codec::{
    MAV_CMD_DO_CHANGE_SPEED, encode_airspeed_command, encode_heading_altitude_setpoint,
};

use super::{AIRSPEED_REPEAT, HeadingAltitudeDemand, UplinkPolicy};

fn policy() -> UplinkPolicy {
    let profile = ValidatedFlightFeelProfile::new(FlightFeelProfile::legacy_compatibility())
        .expect("the legacy profile validates");
    UplinkPolicy::new(profile)
}

fn demand(airspeed_mps: Option<f32>) -> HeadingAltitudeDemand {
    HeadingAltitudeDemand {
        down_m: -300.0,
        heading_rad: 1.5,
        airspeed_mps,
    }
}

/// The MAVLink message id of a frame.
fn message_id(frame: &[u8]) -> u32 {
    u32::from(frame[7]) | (u32::from(frame[8]) << 8) | (u32::from(frame[9]) << 16)
}

/// The command of a COMMAND_LONG frame.
fn command(frame: &[u8]) -> u16 {
    u16::from_le_bytes([frame[10 + 28], frame[10 + 29]])
}

#[test]
fn the_first_demand_sends_the_airspeed_command_and_then_the_setpoint() {
    let mut policy = policy();
    let frames = policy.heading_altitude_frames(Duration::from_millis(2_000), demand(Some(55.0)));
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0], encode_airspeed_command(0, 55.0, 1, 1).to_vec());
    assert_eq!(command(&frames[0]), MAV_CMD_DO_CHANGE_SPEED);
    assert_eq!(
        frames[1],
        encode_heading_altitude_setpoint(1, 2_000, -300.0, 1.5, 1, 1).to_vec()
    );
    assert_eq!(message_id(&frames[1]), 84);
    assert!(policy.airborne(), "a demand to fly opens the stream");
    assert_eq!(policy.heading().heading_rad, 1.5);
}

#[test]
fn an_unchanged_airspeed_is_repeated_only_after_the_repeat_interval() {
    let mut policy = policy();
    let start = Duration::from_secs(10);
    assert_eq!(
        policy
            .heading_altitude_frames(start, demand(Some(55.0)))
            .len(),
        2
    );
    let soon = start + Duration::from_millis(100);
    assert_eq!(
        policy
            .heading_altitude_frames(soon, demand(Some(55.0)))
            .len(),
        1,
        "only the setpoint"
    );
    let changed = policy.heading_altitude_frames(soon, demand(Some(48.0)));
    assert_eq!(changed.len(), 2, "a new airspeed goes out at once");
    assert_eq!(command(&changed[0]), MAV_CMD_DO_CHANGE_SPEED);
    let later = soon + AIRSPEED_REPEAT;
    assert_eq!(
        policy
            .heading_altitude_frames(later, demand(Some(48.0)))
            .len(),
        2,
        "the same airspeed again after the repeat interval"
    );
}

#[test]
fn no_airspeed_sends_no_speed_command() {
    let mut policy = policy();
    let frames = policy.heading_altitude_frames(Duration::from_secs(1), demand(None));
    assert_eq!(frames.len(), 1);
    assert_eq!(message_id(&frames[0]), 84);
}

#[test]
fn nothing_goes_out_in_the_quiet_interval_and_an_arm_forgets_the_airspeed() {
    let mut policy = policy();
    let start = Duration::from_secs(5);
    policy.heading_altitude_frames(start, demand(Some(55.0)));
    policy.arm(start, 0.0);
    assert!(
        policy
            .heading_altitude_frames(start + Duration::from_millis(50), demand(Some(55.0)))
            .is_empty()
    );
    let after_quiet =
        policy.heading_altitude_frames(start + Duration::from_millis(200), demand(Some(55.0)));
    assert_eq!(
        after_quiet.len(),
        2,
        "the airspeed goes out again after the arm"
    );
}
