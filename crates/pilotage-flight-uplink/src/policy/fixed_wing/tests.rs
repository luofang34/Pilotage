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

#[test]
fn a_fixed_wing_velocity_needs_no_climb_input_to_go_out() {
    let mut policy = policy();
    policy.arm(Duration::from_secs(1), 0.2);
    assert_eq!(
        policy.fixed_wing_velocity_frame(Duration::from_millis(1_050), [55.0, 0.0, 0.0], 0.2),
        None,
        "nothing in the quiet interval"
    );
    let frame = policy
        .fixed_wing_velocity_frame(Duration::from_millis(1_200), [55.0, 0.0, 0.0], 0.2)
        .expect("a level cruise demand goes out");
    assert_eq!(message_id(&frame), 84);
    assert_eq!(
        f32::from_le_bytes(frame[10 + 16..10 + 20].try_into().expect("four bytes")),
        55.0,
        "north velocity"
    );
    assert!(policy.airborne(), "the stream is open for the keep-alive");
}

/// How many airspeed commands go out over `seconds` of demands at 20 Hz
/// from `start`.
fn airspeed_commands(policy: &mut UplinkPolicy, start: Duration, seconds: u64) -> usize {
    (0..seconds * 20)
        .map(|tick| start + Duration::from_millis(50 * tick))
        .flat_map(|now| policy.heading_altitude_frames(now, demand(Some(55.0))))
        .filter(|frame| message_id(frame) == 76)
        .count()
}

#[test]
fn an_unsupported_airspeed_is_refused_for_good_and_the_setpoint_flies_on() {
    use super::AirspeedAnswer;
    let mut policy = policy();
    let start = Duration::from_secs(10);
    assert_eq!(airspeed_commands(&mut policy, start, 1), 1);
    // MAV_RESULT_UNSUPPORTED.
    assert_eq!(policy.airspeed_answered(3), AirspeedAnswer::Refused);
    assert!(policy.airspeed_refused());
    assert_eq!(
        policy.airspeed_answered(3),
        AirspeedAnswer::Ignored,
        "said once"
    );
    let later = start + Duration::from_secs(1);
    assert_eq!(airspeed_commands(&mut policy, later, 10), 0, "never again");
    let frames = policy.heading_altitude_frames(later, demand(Some(48.0)));
    assert_eq!(
        frames
            .iter()
            .map(|frame| message_id(frame))
            .collect::<Vec<_>>(),
        [84],
        "a new airspeed is not sent either; the setpoint still is"
    );
    // An arm keeps the refusal: it is the flight controller's.
    policy.arm(later, 0.0);
    let after_arm = later + Duration::from_secs(1);
    assert_eq!(airspeed_commands(&mut policy, after_arm, 2), 0);
    // A vehicle reset may bring another flight controller.
    policy.reset_for_vehicle_reset();
    assert!(!policy.airspeed_refused());
}

#[test]
fn a_denied_airspeed_is_refused_and_a_temporary_rejection_retried() {
    use super::AirspeedAnswer;
    let mut denied = policy();
    airspeed_commands(&mut denied, Duration::from_secs(1), 1);
    // MAV_RESULT_DENIED.
    assert_eq!(denied.airspeed_answered(2), AirspeedAnswer::Refused);

    let mut busy = policy();
    let start = Duration::from_secs(1);
    airspeed_commands(&mut busy, start, 1);
    // MAV_RESULT_TEMPORARILY_REJECTED.
    assert_eq!(busy.airspeed_answered(1), AirspeedAnswer::Retry);
    assert_eq!(
        airspeed_commands(&mut busy, start + Duration::from_secs(1), 3),
        3,
        "once a second"
    );
}

#[test]
fn an_accepted_airspeed_is_not_repeated_until_it_changes() {
    use super::AirspeedAnswer;
    let mut policy = policy();
    let start = Duration::from_secs(1);
    airspeed_commands(&mut policy, start, 1);
    assert_eq!(policy.airspeed_answered(0), AirspeedAnswer::Accepted);
    assert_eq!(
        airspeed_commands(&mut policy, start + Duration::from_secs(1), 5),
        0
    );
    let frames = policy.heading_altitude_frames(start + Duration::from_secs(7), demand(Some(48.0)));
    assert_eq!(
        command(&frames[0]),
        MAV_CMD_DO_CHANGE_SPEED,
        "a new airspeed goes out"
    );
}
