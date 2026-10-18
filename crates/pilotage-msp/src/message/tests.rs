#![allow(clippy::expect_used, clippy::panic)]

//! Payload fixtures are replies captured from Betaflight SITL (API 1.49)
//! with a fresh EEPROM, except where a test names a hand-built layout.

use super::{
    Altitude, ApiVersion, ArmingBlocker, ArmingDisableFlags, Attitude, BatteryState, BoxIds,
    FailsafeConfig, FailsafeProcedure, ModeFlags, ModeRange, PayloadError, RawGps, Status,
    decode_features, decode_multiple, encode_multiple_request, encode_set_mode_range,
    encode_set_raw_rc, feature, permanent_id,
};
use crate::{Decoder, command};

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("fixture hex"))
        .collect()
}

/// The payload of the one frame in a captured reply.
fn reply_payload(frame_hex: &str) -> Vec<u8> {
    let mut frames = Decoder::new().feed(&hex(frame_hex));
    assert_eq!(frames.len(), 1);
    frames.remove(0).expect("captured frame decodes").payload
}

const STATUS_EX_REPLY: &str = "244d3e1b968c0300002700000000000000000400001e040300000000000403003f";

#[test]
fn a_fresh_sitl_status_names_its_arming_blockers() {
    let status = Status::decode(&reply_payload(STATUS_EX_REPLY)).expect("decodes");
    assert_eq!(status.cycle_time_us, 908);
    assert_eq!(status.sensors, 0x27, "acc, baro, mag, gyro");
    assert!(!status.has_gps());
    assert_eq!(status.modes, ModeFlags(0));
    assert_eq!(
        status.arming_disable.blockers(),
        vec![
            ArmingBlocker::RxLoss,
            ArmingBlocker::Angle,
            ArmingBlocker::BootGraceTime
        ]
    );
    assert!(!status.reboot_required);
}

#[test]
fn msp_status_and_status_ex_decode_alike() {
    let status =
        reply_payload("244d3e1b657b0500002700000000000000000000001e0403000000000004030039");
    let decoded = Status::decode(&status).expect("decodes");
    assert_eq!(decoded.arming_disable, ArmingDisableFlags(0x304));
}

#[test]
fn mode_bits_past_thirty_two_come_from_the_extension_bytes() {
    let mut payload = hex("8c0300002700");
    payload.extend(0x0000_0003u32.to_le_bytes()); // boxes 0 and 1
    payload.extend(hex("00000004 00".replace(' ', "").as_str()));
    payload.extend([1, 0b0000_0100]); // one extension byte: box 34
    payload.extend([30]);
    payload.extend(0u32.to_le_bytes());
    let status = Status::decode(&payload).expect("decodes");
    assert!(status.modes.bit(0) && status.modes.bit(1) && status.modes.bit(34));
    assert!(!status.modes.bit(33));
    assert!(status.arming_disable.is_clear());
}

#[test]
fn a_truncated_status_names_the_missing_length() {
    let payload = reply_payload(STATUS_EX_REPLY);
    assert_eq!(
        Status::decode(&payload[..10]),
        Err(PayloadError::Truncated {
            message: "MSP_STATUS",
            len: 10,
            needed: 11,
        })
    );
}

#[test]
fn every_blocker_has_its_own_bit_and_name() {
    let mut seen = 0u32;
    for blocker in ArmingBlocker::ALL {
        assert_eq!(seen & blocker.bit(), 0, "{blocker:?} shares a bit");
        seen |= blocker.bit();
        assert_ne!(blocker.cli_name(), "UNKNOWN");
    }
    assert_eq!(seen, (1 << 30) - 1);
    assert_eq!(ArmingBlocker::ArmSwitch.cli_name(), "ARM_SWITCH");
    assert_eq!(ArmingDisableFlags(1 << 31).unknown_bits(), 1 << 31);
    assert!(ArmingDisableFlags(1 << 31).blockers().is_empty());
}

#[test]
fn captured_zero_attitude_altitude_and_gps_decode() {
    let attitude = Attitude::decode(&reply_payload("244d3e066c0000000000006a")).expect("att");
    assert_eq!(attitude.heading_degrees, 0);
    let altitude = Altitude::decode(&reply_payload("244d3e066d0000000000006b")).expect("alt");
    assert_eq!(altitude.altitude_cm, 0);
    let gps = RawGps::decode(&reply_payload(
        "244d3e176a00000000000000000000000000000000000000000000007d",
    ))
    .expect("gps");
    assert!(!gps.fix);
    assert_eq!(gps.hdop, Some(0));
}

#[test]
fn signed_telemetry_fields_keep_their_sign() {
    let mut attitude = Vec::new();
    for value in [-123i16, 45, 359] {
        attitude.extend(value.to_le_bytes());
    }
    let decoded = Attitude::decode(&attitude).expect("decodes");
    assert_eq!(
        (
            decoded.roll_decidegrees,
            decoded.pitch_decidegrees,
            decoded.heading_degrees
        ),
        (-123, 45, 359)
    );
    let mut altitude = 1_234i32.to_le_bytes().to_vec();
    altitude.extend((-56i16).to_le_bytes());
    assert_eq!(
        Altitude::decode(&altitude).expect("decodes"),
        Altitude {
            altitude_cm: 1_234,
            vario_cm_s: -56
        }
    );
}

#[test]
fn a_gps_reply_from_an_older_api_has_no_dilution_fields() {
    let mut payload = vec![2, 12];
    payload.extend(473_977_420i32.to_le_bytes());
    payload.extend(85_455_940i32.to_le_bytes());
    payload.extend(488u16.to_le_bytes());
    payload.extend(150u16.to_le_bytes());
    payload.extend(900u16.to_le_bytes());
    let gps = RawGps::decode(&payload).expect("decodes");
    assert!(gps.fix);
    assert_eq!(
        (gps.lat_e7, gps.lon_e7, gps.alt_m),
        (473_977_420, 85_455_940, 488)
    );
    assert_eq!((gps.pdop, gps.hdop), (None, None));
}

#[test]
fn the_sitl_reports_no_battery() {
    let battery =
        BatteryState::decode(&reply_payload("244d3e0b8200000000000000000400008d")).expect("bat");
    assert!(!battery.is_present());
}

#[test]
fn captured_box_ids_locate_mode_bits() {
    let boxes = BoxIds::decode(&reply_payload(
        "244d3e1a770001020503061b0b2e38070d131a1e20212224252d313334353940",
    ));
    assert_eq!(&boxes.0[..4], &[0, 1, 2, 5]);
    // POS HOLD (11) is listed at index 7.
    assert!(boxes.is_active(ModeFlags(1 << 7), permanent_id::POSHOLD));
    assert!(!boxes.is_active(ModeFlags(1 << 7), permanent_id::ARM));
    assert!(!boxes.is_active(ModeFlags(u128::MAX), 200), "unlisted box");
}

#[test]
fn mode_ranges_round_trip_and_match_pulses() {
    let arm = ModeRange::from_pulse(permanent_id::ARM, 0, 1_700, 2_100);
    assert_eq!((arm.start_step, arm.end_step), (32, 48));
    assert!(arm.contains(1_800) && !arm.contains(1_500) && !arm.contains(1_000));
    let payload = encode_set_mode_range(3, arm).expect("encodes");
    assert_eq!(payload, vec![3, 0, 0, 32, 48]);
    let ranges = ModeRange::decode_all(&[0, 0, 32, 48, 1, 1, 0, 48, 9]);
    assert_eq!(ranges.len(), 2, "the partial slot is ignored");
    assert_eq!(ranges[0], arm);
    let fresh = ModeRange::decode_all(&reply_payload(
        "244d3e5022000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000072",
    ));
    assert_eq!(fresh.len(), 20);
    assert!(fresh.iter().all(ModeRange::is_unused));
}

#[test]
fn an_out_of_grid_mode_range_is_refused() {
    let range = ModeRange {
        permanent_id: permanent_id::ANGLE,
        aux_channel: 1,
        start_step: 0,
        end_step: 49,
    };
    assert!(matches!(
        encode_set_mode_range(0, range),
        Err(PayloadError::TooMany { .. })
    ));
}

#[test]
fn captured_feature_and_failsafe_config_decode() {
    let features = decode_features(&reply_payload("244d3e042482004010f2")).expect("features");
    assert_ne!(
        features & feature::RX_UDP,
        0,
        "SITL defaults to the UDP receiver"
    );
    assert_eq!(features & feature::RX_MSP, 0);
    assert_ne!(features & feature::GPS, 0);
    let failsafe =
        FailsafeConfig::decode(&reply_payload("244d3e084b0f3ce80300640001fe")).expect("failsafe");
    assert_eq!(failsafe.delay_ds, 15);
    assert_eq!(failsafe.throttle_us, 1_000);
    assert_eq!(failsafe.procedure, FailsafeProcedure::Drop);
}

#[test]
fn api_version_decodes() {
    assert_eq!(
        ApiVersion::decode(&[0, 1, 49]).expect("decodes"),
        ApiVersion {
            protocol: 0,
            major: 1,
            minor: 49
        }
    );
}

#[test]
fn raw_rc_is_little_endian_microseconds() {
    assert_eq!(
        encode_set_raw_rc(&[1_500, 1_000]).expect("encodes"),
        vec![0xDC, 0x05, 0xE8, 0x03]
    );
    assert!(encode_set_raw_rc(&[1_500; 19]).is_err());
}

#[test]
fn a_captured_multiple_reply_splits_by_request_order() {
    let requested = [command::MSP_STATUS_EX, command::MSP_ATTITUDE];
    assert_eq!(
        encode_multiple_request(&requested).expect("v1 commands"),
        vec![0x96, 0x6c]
    );
    let payload = reply_payload(
        "244d3e23e61bc60000002700000000000000000400001e040100000000000403000600000000000021",
    );
    let entries = decode_multiple(&requested, &payload).expect("splits");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].0, command::MSP_STATUS_EX);
    Status::decode(entries[0].1).expect("status entry decodes");
    assert_eq!(entries[1].1.len(), 6);
    Attitude::decode(entries[1].1).expect("attitude entry decodes");
}

#[test]
fn a_multiple_reply_cut_short_returns_the_entries_it_holds() {
    let entries = decode_multiple(&[1, 2, 3], &[1, 0xAA]).expect("splits");
    assert_eq!(entries, vec![(1, &[0xAA][..])]);
    assert!(decode_multiple(&[1], &[5, 0xAA]).is_err());
    assert!(encode_multiple_request(&[0x1001]).is_err());
}
