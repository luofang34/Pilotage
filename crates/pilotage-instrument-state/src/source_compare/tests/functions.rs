#![allow(clippy::expect_used, clippy::panic)]

use super::super::*;
use crate::{AltitudeClass, GeoidModelId, HeadingReference, OriginId, Quat};
use pilotage_alerts::MiscompareFault;

const DEG: f32 = core::f32::consts::PI / 180.0;

#[test]
fn heading_uses_shortest_circular_angle() {
    let p = AirframeSourcePolicy::simulator(MiscompareFault::Heading);
    let mut c = SourceComparator::new(MiscompareFault::Heading);
    let hdg = |src: u8, deg: f32| Candidate {
        source: SourceId(src),
        epoch: SourceEpoch(1),
        source_time_ms: 0,
        receive_time_ms: 0,
        sequence: 0,
        valid: true,
        integrity: IntegrityLevel::None,
        accuracy: 0.0,
        measurement: HeadingMeasure {
            heading_rad: deg * DEG,
            reference: HeadingReference::Magnetic,
        },
    };
    let near = c.step(&[hdg(1, 359.0), hdg(2, 1.0)], &p, 0);
    assert_eq!(
        near.state,
        ComparisonState::Agree,
        "359 and 1 are 2 degrees apart"
    );
    let metric = |deg: f32| HeadingMeasure {
        heading_rad: deg * DEG,
        reference: HeadingReference::Magnetic,
    };
    assert!((metric(359.0).difference(&metric(1.0)) - 2.0 * DEG).abs() < 1e-4);
    assert!((metric(1.0).difference(&metric(359.0)) - 2.0 * DEG).abs() < 1e-4);
}

#[test]
fn attitude_compares_q_and_negated_q_identically() {
    let rot = |ang: f32| Quat {
        w: libm::cosf(ang / 2.0),
        x: 0.0,
        y: 0.0,
        z: libm::sinf(ang / 2.0),
    };
    let neg = |q: Quat| Quat {
        w: -q.w,
        x: -q.x,
        y: -q.y,
        z: -q.z,
    };
    let m = |q: Quat| AttitudeMeasure {
        quat: q,
        frame: FrameTag(1),
    };
    let q1 = Quat::IDENTITY;
    let q2 = rot(1.0 * DEG);
    assert_eq!(
        m(q1).difference(&m(q2)).to_bits(),
        m(q1).difference(&m(neg(q2))).to_bits(),
        "q and -q give a bit-identical geodesic angle"
    );
    let p = AirframeSourcePolicy::simulator(MiscompareFault::Attitude);
    let att = |src: u8, q: Quat| Candidate {
        source: SourceId(src),
        epoch: SourceEpoch(1),
        source_time_ms: 0,
        receive_time_ms: 0,
        sequence: 0,
        valid: true,
        integrity: IntegrityLevel::None,
        accuracy: 0.0,
        measurement: m(q),
    };
    let mut ca = SourceComparator::new(MiscompareFault::Attitude);
    let mut cb = SourceComparator::new(MiscompareFault::Attitude);
    let oa = ca.step(&[att(1, q1), att(2, q2)], &p, 0);
    let ob = cb.step(&[att(1, q1), att(2, neg(q2))], &p, 0);
    assert_eq!(
        oa, ob,
        "the whole comparison is invariant to quaternion sign"
    );
    assert_eq!(
        oa.state,
        ComparisonState::Agree,
        "1 degree is within the band"
    );
}

#[test]
fn incompatible_altitude_datums_are_not_comparable() {
    let p = AirframeSourcePolicy::simulator(MiscompareFault::Altitude);
    let alt = |src: u8, class: AltitudeClass, origin: u32, val: f32| Candidate {
        source: SourceId(src),
        epoch: SourceEpoch(1),
        source_time_ms: 0,
        receive_time_ms: 0,
        sequence: 0,
        valid: true,
        integrity: IntegrityLevel::None,
        accuracy: 0.0,
        measurement: SourceAltitude {
            value_m: val,
            class,
            origin: OriginId(origin),
            model: GeoidModelId::UNDECLARED,
        },
    };
    let mut c = SourceComparator::new(MiscompareFault::Altitude);
    let cross = c.step(
        &[
            alt(1, AltitudeClass::BaroIndicated, 0, 1000.0),
            alt(2, AltitudeClass::Pressure, 0, 1000.0),
        ],
        &p,
        0,
    );
    assert_eq!(
        cross.state,
        ComparisonState::NotComparable,
        "same number, different datum"
    );
    assert_eq!(cross.selected, Some(SourceId(1)));

    let mut c2 = SourceComparator::new(MiscompareFault::Altitude);
    let origins = c2.step(
        &[
            alt(1, AltitudeClass::LocalRelative, 1, 50.0),
            alt(2, AltitudeClass::LocalRelative, 2, 50.0),
        ],
        &p,
        0,
    );
    assert_eq!(
        origins.state,
        ComparisonState::NotComparable,
        "different origins are different data"
    );

    let mut c3 = SourceComparator::new(MiscompareFault::Altitude);
    let same = c3.step(
        &[
            alt(1, AltitudeClass::BaroIndicated, 0, 1000.0),
            alt(2, AltitudeClass::BaroIndicated, 0, 1010.0),
        ],
        &p,
        0,
    );
    assert_eq!(
        same.state,
        ComparisonState::Agree,
        "same datum compares in meters"
    );
}
