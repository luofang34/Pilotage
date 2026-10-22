use super::*;

fn now() -> MonotonicNanos {
    MonotonicNanos::from_nanos(1_000_000)
}

fn stamped_kinematics(role: wire::SourceRole) -> wire::TelemetrySample {
    wire::TelemetrySample {
        flight_procedure: None,
        payload: None,
        avionics: Some(wire::AvionicsState {
            baro_alt_m: 0.0,
            baro_stamp: None,
            pos_n_m: 1.0,
            pos_e_m: 2.0,
            pos_d_m: -3.0,
            kinematics_stamp: Some(wire::MeasurementStamp {
                role: role as i32,
                sequence: 7,
                ..Default::default()
            }),
            ..Default::default()
        }),
        // A pose rides along so a fall-through would have something
        // to upgrade — the tests below prove it never does.
        pose: Some(wire::Pose2d {
            x_m: 9.0,
            y_m: 9.0,
            heading_rad: 1.0,
        }),
        ..Default::default()
    }
}

#[test]
fn an_unreadable_stamped_role_is_refused_not_upgraded_to_pose_truth() {
    let sample = stamped_kinematics(wire::SourceRole::NavigationSolution);
    assert_eq!(ownship_from_wire(&sample, now(), true), None);
    assert_eq!(ownship_from_wire(&sample, now(), false), None);
}

#[test]
fn a_planar_pose_is_truth_only_by_declaration() {
    let sample = wire::TelemetrySample {
        flight_procedure: None,
        payload: None,
        pose: Some(wire::Pose2d {
            x_m: 4.0,
            y_m: 5.0,
            heading_rad: 0.5,
        }),
        ..Default::default()
    };
    let declared = ownship_from_wire(&sample, now(), true).expect("declared planar truth converts");
    assert_eq!(declared.role, TruthRole::SimulationTruth);
    assert_eq!(declared.yaw_rad, Some(0.5));
    assert_eq!(ownship_from_wire(&sample, now(), false), None);
}

#[test]
fn kinematics_without_an_attitude_group_carries_no_yaw() {
    let sample = stamped_kinematics(wire::SourceRole::SimulationTruth);
    let ownship = ownship_from_wire(&sample, now(), false).expect("truth role converts");
    assert_eq!(
        ownship.yaw_rad, None,
        "a substituted zero would read as due north"
    );
    assert_eq!(ownship.ned, [1.0, 2.0, -3.0]);
}

#[test]
fn an_attitude_group_supplies_the_yaw() {
    let mut sample = stamped_kinematics(wire::SourceRole::SimulationTruth);
    let avionics = sample.avionics.as_mut().expect("fixture carries avionics");
    avionics.quat_w = 1.0;
    avionics.attitude_stamp = Some(wire::MeasurementStamp::default());
    let ownship = ownship_from_wire(&sample, now(), false).expect("truth role converts");
    assert_eq!(
        ownship.yaw_rad,
        Some(0.0),
        "identity quaternion is a zero yaw"
    );
}

/// A vehicle's own fix on the wire: WGS-84, local-relative height with
/// a declared origin, authorized position, degraded quality, and a
/// canonical attitude with the same acquisition identity.
fn vehicle_estimate(role: wire::SourceRole) -> wire::TelemetrySample {
    let stamp = wire::MeasurementStamp {
        role: role as i32,
        sequence: 11,
        ..Default::default()
    };
    wire::TelemetrySample {
        flight_procedure: None,
        payload: None,
        avionics: Some(wire::AvionicsState {
            vel_n_mps: 1.2,
            vel_e_mps: -0.5,
            vel_d_mps: 0.3,
            valid_flags: 0b1101,
            quat_w: (0.75_f32).cos(),
            quat_z: (0.75_f32).sin(),
            attitude_stamp: Some(stamp.clone()),
            quality: 1,
            kinematics_stamp: Some(stamp.clone()),
            estimator_status_stamp: Some(stamp.clone()),
            geodetic: Some(wire::GeodeticFix {
                latitude_deg: 40.5442,
                longitude_deg: -74.4565,
                horizontal_datum: 1,
                realization: 0,
                height_m: 110.0,
                vertical_datum: 6,
                local_origin: 77,
                ..Default::default()
            }),
            geodetic_stamp: Some(stamp),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// A conversion that records its inputs: north is the latitude in
/// degrees, east the longitude, down the negated height reading.
fn recording_ned(lat_rad: f64, lon_rad: f64, altitude: OwnshipAltitude) -> [f64; 3] {
    let height = match altitude {
        OwnshipAltitude::AboveAnchor(height_m) => height_m,
        OwnshipAltitude::Ellipsoid(altitude_m) => altitude_m + 1_000.0,
    };
    [lat_rad.to_degrees(), lon_rad.to_degrees(), -height]
}

#[test]
fn the_vehicle_fix_converts_through_the_anchor_with_an_authorized_heading() {
    let sample = vehicle_estimate(wire::SourceRole::OperationalEstimate);
    let ownship = estimate_from_wire(&sample, now(), recording_ned).expect("admitted");
    assert_eq!(ownship.role, TruthRole::OperationalEstimate);
    assert!((ownship.ned[0] - 40.5442).abs() < 1e-9);
    assert!((ownship.ned[1] + 74.4565).abs() < 1e-9);
    assert_eq!(
        ownship.ned[2], -110.0,
        "a local-relative height is height above the anchor"
    );
    assert!((ownship.ned_velocity[0] - 1.2).abs() < 1e-6);
    assert!((ownship.ned_velocity[1] + 0.5).abs() < 1e-6);
    assert!((ownship.ned_velocity[2] - 0.3).abs() < 1e-6);
    assert!((ownship.yaw_rad.expect("canonical heading") - 1.5).abs() < 1e-6);
    assert_eq!(ownship.sequence, 11);
    assert_eq!(ownship.acquired_at, now());
}

#[test]
fn an_ellipsoidal_height_is_placed_directly_and_other_datums_are_refused() {
    let mut sample = vehicle_estimate(wire::SourceRole::OperationalEstimate);
    let fix = sample
        .avionics
        .as_mut()
        .and_then(|avionics| avionics.geodetic.as_mut())
        .expect("fixture carries a fix");
    fix.vertical_datum = 1;
    fix.height_m = 600.0;
    let ownship = estimate_from_wire(&sample, now(), recording_ned).expect("admitted");
    assert_eq!(
        ownship.ned[2], -1_600.0,
        "ellipsoidal height reaches the conversion"
    );
    for (datum, origin) in [(2_u32, 0_u64), (3, 0), (5, 0), (6, 0), (0, 0), (99, 0)] {
        let mut sample = vehicle_estimate(wire::SourceRole::OperationalEstimate);
        let fix = sample
            .avionics
            .as_mut()
            .and_then(|avionics| avionics.geodetic.as_mut())
            .expect("fixture carries a fix");
        fix.vertical_datum = datum;
        fix.local_origin = origin;
        assert!(
            estimate_from_wire(&sample, now(), recording_ned).is_none(),
            "vertical datum {datum} with origin {origin} has no declared conversion"
        );
    }
}

#[test]
fn an_unauthorized_unusable_or_wrongly_labeled_estimate_is_refused() {
    type Mutation = fn(&mut wire::AvionicsState);
    let cases: [(&str, Mutation); 6] = [
        ("no estimator status", |a| a.estimator_status_stamp = None),
        ("position not authorized", |a| a.valid_flags = 0b0011),
        ("unusable quality", |a| a.quality = 2),
        ("no kinematics stamp", |a| a.kinematics_stamp = None),
        ("no fix", |a| a.geodetic = None),
        ("non-WGS-84 fix", |a| {
            a.geodetic.as_mut().expect("fix").horizontal_datum = 2;
        }),
    ];
    for (label, mutate) in cases {
        let mut sample = vehicle_estimate(wire::SourceRole::OperationalEstimate);
        mutate(sample.avionics.as_mut().expect("fixture carries avionics"));
        assert!(
            estimate_from_wire(&sample, now(), recording_ned).is_none(),
            "{label} must be refused"
        );
    }
    for role in [
        wire::SourceRole::SimulationTruth,
        wire::SourceRole::FcState,
        wire::SourceRole::NavigationSolution,
    ] {
        let sample = vehicle_estimate(role);
        assert!(
            estimate_from_wire(&sample, now(), recording_ned).is_none(),
            "{role:?} is not the estimate lane"
        );
    }
}

#[test]
fn geographic_guidance_refuses_unknown_stale_or_unauthorized_attitude() {
    type Mutation = fn(&mut wire::AvionicsState);
    let cases: [Mutation; 5] = [
        |a| {
            a.attitude_stamp = None;
            a.reported_euler = Some(wire::ReportedEulerAttitude {
                yaw_rad: 1.5,
                stamp: a.geodetic_stamp.clone(),
                ..Default::default()
            });
        },
        |a| a.valid_flags &= !1,
        |a| a.attitude_stamp.as_mut().expect("attitude").sequence = 10,
        |a| a.quat_w = f32::NAN,
        |a| {
            a.quat_w = 0.0;
            a.quat_z = 0.0;
        },
    ];
    for mutate in cases {
        let mut sample = vehicle_estimate(wire::SourceRole::OperationalEstimate);
        mutate(sample.avionics.as_mut().expect("avionics"));
        assert!(estimate_from_wire(&sample, now(), recording_ned).is_none());
    }
}
