use pilotage_adapter_api::{AvionicsAttitudeSample, SourceRole};
use pilotage_geo::{HorizontalDatum, VerticalDatum};

use super::{
    TestResult,
    harness::{Harness, captured_hex_frames, packets, seal_duml},
};
use crate::flight::Mini3PositionReport;

/// The OSD payload starts after the 11-byte DUML header.
const PAYLOAD: usize = 11;
/// Controller-state bits: satellite positioning in use, signal level 5.
const GPS_GOOD: u32 = 0x8000 | (5 << 18);
/// Controller-state bit: motors running.
const MOTOR_ON: u32 = 0x08;
const LAT_DEG: f64 = 40.5442;
const LON_DEG: f64 = -74.4565;

struct Osd {
    sequence: u16,
    lat_deg: f64,
    lon_deg: f64,
    height_dm: i16,
    velocity_dmps: [i16; 3],
    flags: u32,
    satellites: u8,
}

impl Osd {
    fn good(sequence: u16) -> Self {
        Self {
            sequence,
            lat_deg: LAT_DEG,
            lon_deg: LON_DEG,
            height_dm: 1_100,
            velocity_dmps: [12, -5, 3],
            flags: GPS_GOOD,
            satellites: 14,
        }
    }

    fn bytes(&self) -> Vec<u8> {
        let mut bytes = captured_hex_frames(include_str!(
            "../airframe/fixtures/mini3-reported-attitude.hex"
        ))
        .remove(0);
        bytes[6..8].copy_from_slice(&self.sequence.to_le_bytes());
        bytes[PAYLOAD..PAYLOAD + 8].copy_from_slice(&self.lon_deg.to_radians().to_le_bytes());
        bytes[PAYLOAD + 8..PAYLOAD + 16].copy_from_slice(&self.lat_deg.to_radians().to_le_bytes());
        bytes[PAYLOAD + 16..PAYLOAD + 18].copy_from_slice(&self.height_dm.to_le_bytes());
        for (index, component) in self.velocity_dmps.iter().enumerate() {
            let at = PAYLOAD + 18 + 2 * index;
            bytes[at..at + 2].copy_from_slice(&component.to_le_bytes());
        }
        bytes[PAYLOAD + 32..PAYLOAD + 36].copy_from_slice(&self.flags.to_le_bytes());
        bytes[PAYLOAD + 36] = self.satellites;
        seal_duml(&mut bytes);
        bytes
    }
}

fn feed(harness: &mut Harness, osd: &Osd) -> TestResult {
    let bytes = osd.bytes();
    let parsed = packets(&bytes);
    assert_eq!(parsed.len(), 1);
    harness.feed(&crate::logiclink::encode_control(&parsed[0]))?;
    Ok(())
}

#[test]
fn bench_capture_carries_no_satellite_solution() {
    let bytes = captured_hex_frames(include_str!(
        "../airframe/fixtures/mini3-reported-attitude.hex"
    ))
    .remove(0);
    let report = Mini3PositionReport::from_push(&packets(&bytes)[0]).unwrap();
    assert_eq!(report.gps_level, 0);
    assert!(!report.gps_used);
    assert_eq!(report.satellites, 5);
    assert_eq!(report.latitude_rad, 0.0);
    assert_eq!(report.longitude_rad, 0.0);
    assert!(report.fix().is_none());
}

#[test]
fn a_fix_publishes_position_and_velocity_under_the_estimate_role() -> TestResult {
    let mut harness = Harness::mobile();
    feed(&mut harness, &Osd::good(1))?;
    harness.step(0);
    let batch = harness.telemetry(0);
    let sample = &batch.samples[0];
    assert!((sample.speed.unwrap() - 1.3).abs() < 1e-9);
    assert!(sample.pose.is_none());
    let avionics = sample.avionics.unwrap();
    assert!(matches!(
        avionics.attitude,
        Some(AvionicsAttitudeSample::ReportedEuler { .. })
    ));
    let fix = avionics.geodetic.unwrap();
    assert!((fix.position.latitude_deg - LAT_DEG).abs() < 1e-9);
    assert!((fix.position.longitude_deg - LON_DEG).abs() < 1e-9);
    assert_eq!(fix.position.horizontal_datum, HorizontalDatum::Wgs84);
    assert_eq!(fix.position.vertical.datum, VerticalDatum::LocalRelative);
    assert!((fix.position.vertical.height_m - 110.0).abs() < 1e-9);
    assert!(fix.position.vertical.origin.is_declared());
    assert!(fix.quality.stated_horizontal_mm().is_none());
    assert_eq!(fix.stamp.role, SourceRole::OperationalEstimate);
    assert_eq!(fix.stamp.source_id, 0x0e_03_43);
    let kinematics = avionics.kinematics.unwrap();
    assert_eq!(kinematics.pos_ned_m, [0.0, 0.0, -110.0]);
    assert!((kinematics.vel_ned_mps[0] - 1.2).abs() < 1e-6);
    assert!((kinematics.vel_ned_mps[1] + 0.5).abs() < 1e-6);
    assert!((kinematics.vel_ned_mps[2] - 0.3).abs() < 1e-6);
    assert_eq!(kinematics.stamp, fix.stamp);
    assert_eq!(avionics.estimator_status_stamp, Some(fix.stamp));
    assert_eq!(avionics.valid_flags, 0b1100);
    assert_eq!(avionics.quality, 0);
    Ok(())
}

#[test]
fn later_fixes_measure_from_the_first_fix_of_the_binding() -> TestResult {
    let mut harness = Harness::mobile();
    feed(&mut harness, &Osd::good(1))?;
    harness.step(0);
    harness.telemetry(0);
    let moved = Osd {
        lat_deg: LAT_DEG + 0.001,
        lon_deg: LON_DEG + 0.001,
        height_dm: 1_050,
        ..Osd::good(2)
    };
    feed(&mut harness, &moved)?;
    harness.step(100);
    let batch = harness.telemetry(100);
    let kinematics = batch.samples[0].avionics.unwrap().kinematics.unwrap();
    // WGS-84 meridional and prime-vertical arc lengths at 40.5442 degrees.
    assert!((f64::from(kinematics.pos_ned_m[0]) - 111.05).abs() < 0.2);
    assert!((f64::from(kinematics.pos_ned_m[1]) - 84.71).abs() < 0.2);
    assert!((f64::from(kinematics.pos_ned_m[2]) + 105.0).abs() < 1e-4);
    Ok(())
}

#[test]
fn satellite_use_sets_authorization_and_signal_level_sets_quality() -> TestResult {
    for (flags, valid_flags, quality) in [
        (0x8000 | (3 << 18), 0b1100, 1),
        (5 << 18, 0, 1),
        (3 << 18, 0, 1),
    ] {
        let mut harness = Harness::mobile();
        feed(
            &mut harness,
            &Osd {
                flags,
                ..Osd::good(1)
            },
        )?;
        harness.step(0);
        let batch = harness.telemetry(0);
        let avionics = batch.samples[0].avionics.unwrap();
        assert!(avionics.geodetic.is_some() && avionics.kinematics.is_some());
        assert!(avionics.estimator_status_stamp.is_some());
        assert_eq!(avionics.valid_flags, valid_flags);
        assert_eq!(avionics.quality, quality);
    }
    Ok(())
}

#[test]
fn no_solution_or_impossible_coordinates_publish_no_fix() -> TestResult {
    let cases = [
        Osd {
            flags: 0,
            ..Osd::good(1)
        },
        Osd {
            lat_deg: 100.0,
            ..Osd::good(1)
        },
        Osd {
            lon_deg: f64::NAN,
            ..Osd::good(1)
        },
        Osd {
            lat_deg: 0.0,
            lon_deg: 0.0,
            ..Osd::good(1)
        },
    ];
    for osd in &cases {
        let mut harness = Harness::mobile();
        feed(&mut harness, osd)?;
        harness.step(0);
        let batch = harness.telemetry(0);
        let sample = &batch.samples[0];
        assert!(sample.speed.is_none());
        let avionics = sample.avionics.unwrap();
        assert!(avionics.geodetic.is_none() && avionics.kinematics.is_none());
        assert!(avionics.estimator_status_stamp.is_none());
        assert_eq!(avionics.valid_flags, 0);
        assert_eq!(avionics.quality, 2);
        assert!(avionics.attitude.is_some());
    }
    Ok(())
}

#[test]
fn a_motor_start_or_a_new_binding_changes_the_height_origin() -> TestResult {
    let mut harness = Harness::mobile();
    feed(&mut harness, &Osd::good(1))?;
    harness.step(0);
    let first = harness.telemetry(0).samples[0].avionics.unwrap();
    let first_origin = first.geodetic.unwrap().position.vertical.origin;

    feed(
        &mut harness,
        &Osd {
            flags: GPS_GOOD | MOTOR_ON,
            ..Osd::good(2)
        },
    )?;
    harness.step(100);
    let started = harness.telemetry(100).samples[0].avionics.unwrap();
    let started_origin = started.geodetic.unwrap().position.vertical.origin;
    assert_ne!(started_origin, first_origin);
    // The horizontal anchor is unchanged by a motor start.
    assert_eq!(started.kinematics.unwrap().pos_ned_m[..2], [0.0, 0.0]);

    // Another model on the OSD route closes the binding.
    let mut other = Osd::good(3).bytes();
    other[PAYLOAD + 48] = 1;
    seal_duml(&mut other);
    harness.feed(&crate::logiclink::encode_control(&packets(&other)[0]))?;
    harness.step(200);
    assert!(harness.telemetry(200).samples.is_empty());

    let moved = Osd {
        lat_deg: LAT_DEG + 0.001,
        flags: GPS_GOOD | MOTOR_ON,
        ..Osd::good(4)
    };
    feed(&mut harness, &moved)?;
    harness.step(300);
    let rebound = harness.telemetry(300).samples[0].avionics.unwrap();
    let rebound_origin = rebound.geodetic.unwrap().position.vertical.origin;
    assert_ne!(rebound_origin, started_origin);
    assert_ne!(rebound_origin, first_origin);
    // The new binding anchors at its own first fix.
    assert_eq!(rebound.kinematics.unwrap().pos_ned_m[..2], [0.0, 0.0]);
    Ok(())
}
