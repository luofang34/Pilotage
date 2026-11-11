use pilotage_protocol::{AltitudeReference, HeadingAltitudeIntent};

use super::super::{MAGIC_V2, compute_crc, encode_command_long, encode_velocity_setpoint};
use super::{
    HEADING_ALTITUDE_TYPE_MASK, LocalDownError, MAV_CMD_DO_CHANGE_SPEED, encode_airspeed_command,
    encode_heading_altitude_setpoint, local_down_m,
};

/// Where the payload starts in a MAVLink 2 frame.
const PAYLOAD: usize = 10;

fn f32_at(frame: &[u8], payload_offset: usize) -> f32 {
    let at = PAYLOAD + payload_offset;
    f32::from_le_bytes(frame[at..at + 4].try_into().expect("four bytes"))
}

fn intent(altitude_m: f32, reference: AltitudeReference) -> HeadingAltitudeIntent {
    HeadingAltitudeIntent {
        heading_rad: 0.5,
        altitude_m,
        reference,
        airspeed_mps: None,
    }
}

#[test]
fn the_setpoint_carries_only_the_down_coordinate_and_the_yaw() {
    let frame = encode_heading_altitude_setpoint(9, 1234, -350.0, 1.25, 7, 1);
    assert_eq!(frame[0], MAGIC_V2);
    assert_eq!(usize::from(frame[1]), 53, "payload length");
    assert_eq!(frame[4], 9, "sequence");
    assert_eq!(&frame[7..10], &[84, 0, 0], "SET_POSITION_TARGET_LOCAL_NED");
    assert_eq!(
        u32::from_le_bytes(frame[10..14].try_into().expect("four bytes")),
        1234
    );
    assert_eq!(f32_at(&frame, 12), -350.0, "down coordinate");
    assert_eq!(f32_at(&frame, 40), 1.25, "yaw");
    // Every value the mask ignores is zero on the wire.
    for ignored in [4, 8, 16, 20, 24, 28, 32, 36, 44] {
        assert_eq!(f32_at(&frame, ignored), 0.0, "payload offset {ignored}");
    }
    let mask = u16::from_le_bytes(frame[58..60].try_into().expect("two bytes"));
    assert_eq!(mask, HEADING_ALTITUDE_TYPE_MASK);
    assert_eq!(mask, 2555);
    assert_eq!(mask & 4, 0, "the down coordinate is not ignored");
    assert_eq!(mask & 1024, 0, "the yaw is not ignored");
    assert_eq!(
        &frame[60..63],
        &[7, 1, 1],
        "target, component, LOCAL_NED frame"
    );
    let crc = compute_crc(&frame[1..63], 143);
    assert_eq!(&frame[63..65], &crc.to_le_bytes());
}

#[test]
fn the_setpoint_differs_from_a_velocity_setpoint_only_in_what_it_commands() {
    let heading_altitude = encode_heading_altitude_setpoint(3, 77, 0.0, 0.75, 1, 1);
    let velocity = encode_velocity_setpoint(3, 77, [0.0; 3], 0.75, 1, 1);
    let differing: Vec<usize> = (0..63)
        .filter(|index| heading_altitude[*index] != velocity[*index])
        .collect();
    // The two masks have the same high byte, so one byte differs.
    assert_eq!(differing, vec![58], "only the type mask differs");
}

#[test]
fn the_airspeed_command_is_a_change_speed_command_for_airspeed() {
    let frame = encode_airspeed_command(4, 52.0, 7, 1);
    assert_eq!(
        frame,
        encode_command_long(4, 178, [0.0, 52.0, -1.0, 0.0, 0.0, 0.0, 0.0], 7, 1)
    );
    assert_eq!(MAV_CMD_DO_CHANGE_SPEED, 178);
    assert_eq!(f32_at(&frame, 0), 0.0, "speed type: airspeed");
    assert_eq!(f32_at(&frame, 4), 52.0, "the airspeed");
    assert_eq!(f32_at(&frame, 8), -1.0, "throttle: no change");
    assert_eq!(
        u16::from_le_bytes(frame[38..40].try_into().expect("two bytes")),
        178
    );
}

#[test]
fn an_altitude_above_the_origin_is_its_height_with_the_sign_changed() {
    let above_origin = intent(120.0, AltitudeReference::LocalOrigin);
    assert_eq!(local_down_m(&above_origin, None), Ok(-120.0));
    // The origin height takes no part in an altitude above the origin.
    assert_eq!(local_down_m(&above_origin, Some(431.0)), Ok(-120.0));
    let below_origin = intent(-15.5, AltitudeReference::LocalOrigin);
    assert_eq!(local_down_m(&below_origin, None), Ok(15.5));
}

#[test]
fn an_altitude_above_the_ellipsoid_needs_the_height_of_the_origin() {
    let ellipsoidal = intent(900.0, AltitudeReference::Ellipsoid);
    assert_eq!(local_down_m(&ellipsoidal, Some(431.0)), Ok(-469.0));
    assert_eq!(local_down_m(&ellipsoidal, Some(1_000.0)), Ok(100.0));
    assert_eq!(
        local_down_m(&ellipsoidal, None),
        Err(LocalDownError::OriginHeightUnknown { altitude_m: 900.0 })
    );
}

#[test]
fn a_down_coordinate_that_is_not_finite_is_refused() {
    let ellipsoidal = intent(900.0, AltitudeReference::Ellipsoid);
    for origin in [f64::NAN, f64::INFINITY] {
        assert_eq!(
            local_down_m(&ellipsoidal, Some(origin)),
            Err(LocalDownError::NotFinite { altitude_m: 900.0 })
        );
    }
}
