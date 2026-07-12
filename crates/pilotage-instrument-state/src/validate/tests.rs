#![allow(clippy::expect_used, clippy::panic)]

use super::{GroupFault, QUAT_NORM_TOLERANCE, validate_quat, validate_state};
use crate::aircraft::{
    AirData, AircraftState, Attitude, EstimateQuality, Kinematics, NavData, NavFromTo, NavSource,
    Selections, SnapshotCoherence, SnapshotMeta, Stamped, Wind,
};
use pilotage_frames::Quat;

const BAD: [f32; 3] = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY];

fn quat_scaled(scale: f32) -> Quat {
    Quat {
        w: scale,
        x: 0.0,
        y: 0.0,
        z: 0.0,
    }
}

fn stamped<T>(data: T) -> Stamped<T> {
    Stamped {
        data: Some(data),
        age_ms: Some(10.0),
    }
}

fn trusted_full_state() -> AircraftState {
    AircraftState {
        attitude: stamped(Attitude {
            quat: Quat::IDENTITY,
            rates_rps: [0.0; 3],
        }),
        kinematics: stamped(Kinematics {
            pos_ned_m: [1.0, 2.0, -3.0],
            vel_ned_mps: [4.0, 5.0, -0.5],
        }),
        air: stamped(AirData {
            ias_mps: Some(40.0),
            baro_setting_hpa: Some(1013.25),
        }),
        nav: stamped(NavData {
            source: NavSource::Gps,
            course_rad: 0.3,
            cdi_dots: -1.0,
            fromto: NavFromTo::To,
            vdev_dots: Some(0.2),
            dist_nm: Some(12.5),
        }),
        wind: stamped(Wind {
            from_rad: 1.0,
            speed_mps: 5.0,
        }),
        quality: EstimateQuality::Good,
        valid: crate::aircraft::ValidFlags {
            attitude: true,
            rates: true,
            position: true,
            velocity: true,
            ..Default::default()
        },
        ..AircraftState::default()
    }
}

#[test]
fn trusted_state_reports_no_faults() {
    assert_eq!(validate_state(&trusted_full_state()), Default::default());
}

#[test]
fn quat_tolerance_boundaries_normalize_or_fail() {
    // Inside the tolerance: normalized to unit length.
    let inside = validate_quat(quat_scaled(1.0 + QUAT_NORM_TOLERANCE * 0.99)).expect("normalizes");
    assert!((inside.w - 1.0).abs() < 1e-6);
    let below = validate_quat(quat_scaled(1.0 - QUAT_NORM_TOLERANCE * 0.99)).expect("normalizes");
    assert!((below.w - 1.0).abs() < 1e-6);

    // Outside the tolerance in either direction: a gross error fails
    // rather than being silently repaired.
    for scale in [
        1.0 + QUAT_NORM_TOLERANCE * 1.5,
        1.0 - QUAT_NORM_TOLERANCE * 1.5,
        0.0,
        10.0,
    ] {
        assert_eq!(
            validate_quat(quat_scaled(scale)),
            Err(GroupFault::QuatNorm),
            "scale {scale}"
        );
    }
    for bad in BAD {
        assert_eq!(
            validate_quat(Quat {
                w: bad,
                x: 0.0,
                y: 0.0,
                z: 0.0,
            }),
            Err(GroupFault::NonFinite),
            "component {bad}"
        );
    }
}

#[test]
fn non_finite_values_fault_exactly_their_own_group() {
    for bad in BAD {
        let mut s = trusted_full_state();
        if let Some(att) = s.attitude.data.as_mut() {
            att.rates_rps[1] = bad;
        }
        let report = validate_state(&s);
        assert_eq!(report.rates, Some(GroupFault::NonFinite), "{bad}");
        // Isolation: no other group is tainted by the rates fault.
        assert_eq!(report.attitude, None);
        assert_eq!(report.position, None);
        assert_eq!(report.air, None);
        assert_eq!(report.nav, None);
        assert_eq!(report.wind, None);

        let mut s = trusted_full_state();
        if let Some(kin) = s.kinematics.data.as_mut() {
            kin.pos_ned_m[2] = bad;
        }
        let report = validate_state(&s);
        assert_eq!(report.position, Some(GroupFault::NonFinite));
        assert_eq!(report.velocity, None);

        let mut s = trusted_full_state();
        if let Some(air) = s.air.data.as_mut() {
            air.baro_setting_hpa = Some(bad);
        }
        assert_eq!(validate_state(&s).air, Some(GroupFault::NonFinite));

        let mut s = trusted_full_state();
        if let Some(nav) = s.nav.data.as_mut() {
            nav.cdi_dots = bad;
        }
        assert_eq!(validate_state(&s).nav, Some(GroupFault::NonFinite));

        let mut s = trusted_full_state();
        if let Some(wind) = s.wind.data.as_mut() {
            wind.speed_mps = bad;
        }
        assert_eq!(validate_state(&s).wind, Some(GroupFault::NonFinite));

        let mut s = trusted_full_state();
        s.selections.heading_bug_rad = bad;
        assert_eq!(validate_state(&s).selections, Some(GroupFault::NonFinite));
    }
}

#[test]
fn finite_extrema_are_not_faults() {
    let mut s = trusted_full_state();
    if let Some(kin) = s.kinematics.data.as_mut() {
        kin.pos_ned_m = [f32::MAX, f32::MIN, -f32::MAX];
    }
    assert_eq!(validate_state(&s).position, None);
}

#[test]
fn optional_absence_is_not_a_fault() {
    let mut s = trusted_full_state();
    if let Some(air) = s.air.data.as_mut() {
        air.ias_mps = None;
        air.baro_setting_hpa = None;
    }
    if let Some(nav) = s.nav.data.as_mut() {
        nav.vdev_dots = None;
        nav.dist_nm = None;
    }
    s.selections.altitude_sel_m = None;
    let report = validate_state(&s);
    assert_eq!(report.air, None);
    assert_eq!(report.nav, None);
    assert_eq!(report.selections, None);
}

#[test]
fn unknown_enums_and_quality_are_typed_faults() {
    let mut s = trusted_full_state();
    if let Some(nav) = s.nav.data.as_mut() {
        nav.source = NavSource::Unknown;
    }
    assert_eq!(validate_state(&s).nav, Some(GroupFault::UnknownEnum));

    let mut s = trusted_full_state();
    if let Some(nav) = s.nav.data.as_mut() {
        nav.fromto = NavFromTo::Unknown;
    }
    assert_eq!(validate_state(&s).nav, Some(GroupFault::UnknownEnum));

    let mut s = trusted_full_state();
    s.quality = EstimateQuality::Unknown;
    assert_eq!(validate_state(&s).quality, Some(GroupFault::UnknownQuality));

    let mut s = trusted_full_state();
    s.snapshot = SnapshotMeta {
        generation: 1,
        coherence: SnapshotCoherence::Unknown,
    };
    assert_eq!(validate_state(&s).coherence, Some(GroupFault::UnknownEnum));
}

#[test]
fn absent_groups_are_not_validated() {
    let empty = AircraftState {
        selections: Selections::default(),
        ..AircraftState::default()
    };
    let report = validate_state(&empty);
    // Absence resolves Missing downstream; it is not an integrity fault
    // (but the undeclared quality still is).
    assert_eq!(report.attitude, None);
    assert_eq!(report.nav, None);
    assert_eq!(report.quality, Some(GroupFault::UnknownQuality));
}
