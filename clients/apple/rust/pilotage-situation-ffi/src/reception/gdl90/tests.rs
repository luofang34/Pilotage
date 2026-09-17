#![allow(clippy::expect_used, clippy::panic)]

use aero_link::core::{
    GDL90_AHRS_LEN, GDL90_FOREFLIGHT_AHRS_LEN, Gdl90AhrsReport, encode_gdl90_ahrs,
    encode_gdl90_foreflight_ahrs, frame_gdl90_message,
};

use crate::{Gdl90HeadingReferenceValue, RadioDomainSession};

const TRAFFIC: [u8; 28] = [
    0x14, 0x00, 0xab, 0x45, 0x49, 0x1f, 0xef, 0x15, 0xa8, 0x89, 0x78, 0x0f, 0x09, 0xa9, 0x07, 0xb0,
    0x01, 0x20, 0x01, 0x4e, 0x38, 0x32, 0x35, 0x56, 0x20, 0x20, 0x20, 0x00,
];

#[test]
fn split_appliance_frame_reaches_the_shared_traffic_engine() {
    let session = RadioDomainSession::new().expect("radio session must start");
    let frame = framed(&TRAFFIC);
    let split = frame.len() / 2;
    let first = session
        .accept_gdl90_chunk(frame[..split].to_vec(), 42, 3, 1_000_000)
        .expect("first chunk must be accepted");
    assert_eq!(first.valid_frames, 0);
    let second = session
        .accept_gdl90_chunk(frame[split..].to_vec(), 42, 3, 1_000_100)
        .expect("second chunk must be accepted");
    assert_eq!(second.valid_frames, 1);
    assert_eq!(second.crc_errors, 0);
    assert_eq!(second.traffic_reports, 1);
    assert_eq!(second.records.traffic_observations, 1);
    assert_eq!(second.records.track_records.len(), 1);
    let record = &second.records.track_records[0];
    assert!(record.contains("installed_avionics"));
    assert!(record.contains("panel_fused"));
    assert!(record.contains("N825V"));
}

#[test]
fn appliance_attitude_and_baro_keep_heading_absent() {
    let session = RadioDomainSession::new().expect("radio session must start");
    let report = Gdl90AhrsReport {
        roll_degrees: Some(4.5),
        pitch_degrees: Some(-2.0),
        pressure_altitude_feet: Some(1_234.0),
        vertical_speed_feet_per_minute: Some(320.0),
        ..Gdl90AhrsReport::default()
    };
    let mut legacy = [0_u8; GDL90_AHRS_LEN];
    encode_gdl90_ahrs(report, &mut legacy).expect("legacy AHRS must encode");
    let mut foreflight = [0_u8; GDL90_FOREFLIGHT_AHRS_LEN];
    encode_gdl90_foreflight_ahrs(report, &mut foreflight).expect("ForeFlight AHRS must encode");
    let mut stream = framed(&legacy);
    stream.extend(framed(&foreflight));
    let batch = session
        .accept_gdl90_chunk(stream, 42, 1, 2_000_000)
        .expect("AHRS stream must be accepted");
    let navigation = batch.navigation.expect("navigation must be present");
    assert_eq!(navigation.roll_degrees, Some(4.5));
    assert_eq!(navigation.pitch_degrees, Some(-2.0));
    assert_eq!(navigation.pressure_altitude_feet, Some(1_234.0));
    assert_eq!(navigation.vertical_speed_feet_per_minute, Some(320.0));
    assert_eq!(navigation.heading_degrees, None);
    assert_eq!(navigation.heading_reference, None);
}

#[test]
fn appliance_preserves_true_heading_reference() {
    let session = RadioDomainSession::new().expect("radio session must start");
    let report = Gdl90AhrsReport {
        heading: Some(aero_link::core::Gdl90Heading {
            degrees: 123.4,
            reference: aero_link::core::Gdl90HeadingReference::True,
        }),
        ..Gdl90AhrsReport::default()
    };
    let mut message = [0_u8; GDL90_FOREFLIGHT_AHRS_LEN];
    encode_gdl90_foreflight_ahrs(report, &mut message).expect("AHRS must encode");
    let batch = session
        .accept_gdl90_chunk(framed(&message), 42, 1, 3_000_000)
        .expect("AHRS frame must be accepted");
    let navigation = batch.navigation.expect("navigation must be present");
    assert!((navigation.heading_degrees.expect("heading must be present") - 123.4).abs() < 0.001);
    assert_eq!(
        navigation.heading_reference,
        Some(Gdl90HeadingReferenceValue::TrueNorth)
    );
}

fn framed(message: &[u8]) -> Vec<u8> {
    let mut output = vec![0_u8; message.len() * 2 + 8];
    let length = frame_gdl90_message(message, &mut output).expect("message must frame");
    output.truncate(length);
    output
}
