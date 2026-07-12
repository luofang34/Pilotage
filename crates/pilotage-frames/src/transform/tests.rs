#![allow(clippy::expect_used, clippy::panic)]

use super::{FrameTransform, ROTATION_NORM_TOLERANCE};
use crate::error::FrameError;
use crate::frame::FrameId;
use crate::rotation::Quat;
use crate::time::{ClockDomain, Epoch, TimeScale};

pub(crate) fn epoch(nanos: u64) -> Epoch {
    Epoch {
        clock: ClockDomain::Simulation,
        scale: TimeScale::Monotonic,
        nanos,
    }
}

pub(crate) fn yaw_quat(half_turns_of_pi: f64) -> Quat {
    let h = half_turns_of_pi * core::f64::consts::PI / 2.0;
    Quat {
        w: h.cos() as f32,
        x: 0.0,
        y: 0.0,
        z: h.sin() as f32,
    }
}

fn t(from: FrameId, to: FrameId, q: Quat, tr: [f64; 3]) -> FrameTransform {
    FrameTransform::new(from, to, epoch(7), q, tr).expect("valid transform")
}

fn close(a: [f64; 3], b: [f64; 3]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6)
}

#[test]
fn identity_and_inverse_laws_hold() {
    let ab = t(
        FrameId::Ned,
        FrameId::Ecef,
        yaw_quat(0.31),
        [1.0e7, -2.0e6, 3.0e5],
    );
    let round = ab.then(&ab.inverse()).expect("A→B→A composes");
    assert_eq!(round.from_frame(), FrameId::Ned);
    assert_eq!(round.to_frame(), FrameId::Ned);
    assert!(close(round.translation_m(), [0.0, 0.0, 0.0]));
    let p = [123.0, -456.0, 789.0];
    assert!(close(round.rotation().rotate(p), p));
}

#[test]
fn composition_is_associative_within_tolerance() {
    let ab = t(FrameId::Body, FrameId::Ned, yaw_quat(0.25), [1.0, 2.0, 3.0]);
    let bc = t(
        FrameId::Ned,
        FrameId::Ecef,
        yaw_quat(0.5),
        [-4.0, 5.0, -6.0],
    );
    let cd = t(
        FrameId::Ecef,
        FrameId::Eci,
        yaw_quat(0.75),
        [7.0, -8.0, 9.0],
    );
    let left = ab.then(&bc).expect("ab.bc").then(&cd).expect("(ab.bc).cd");
    let right = ab.then(&bc.then(&cd).expect("bc.cd")).expect("ab.(bc.cd)");
    let p = [10.0, 20.0, 30.0];
    let lp = left.rotation().rotate(p);
    let rp = right.rotation().rotate(p);
    // The rotation kernel is f32: two grouping orders take different
    // rounding paths, so associativity holds to f32 precision at these
    // magnitudes, not to f64 exactness.
    let assoc = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4);
    assert!(assoc(lp, rp));
    assert!(assoc(left.translation_m(), right.translation_m()));
}

#[test]
fn junction_frame_mismatch_is_typed() {
    let ab = t(FrameId::Body, FrameId::Ned, Quat::IDENTITY, [0.0; 3]);
    let cd = t(FrameId::Ecef, FrameId::Eci, Quat::IDENTITY, [0.0; 3]);
    assert_eq!(
        ab.then(&cd),
        Err(FrameError::FrameMismatch {
            expected: FrameId::Ned,
            found: FrameId::Ecef,
        })
    );
}

#[test]
fn epoch_clock_and_scale_mismatches_are_distinct_errors() {
    let base = FrameTransform::new(
        FrameId::Ned,
        FrameId::Ecef,
        epoch(7),
        Quat::IDENTITY,
        [0.0; 3],
    )
    .expect("valid");
    let later = FrameTransform::new(
        FrameId::Ecef,
        FrameId::Eci,
        epoch(8),
        Quat::IDENTITY,
        [0.0; 3],
    )
    .expect("valid");
    assert!(matches!(
        base.then(&later),
        Err(FrameError::EpochMismatch { .. })
    ));

    let other_clock = FrameTransform::new(
        FrameId::Ecef,
        FrameId::Eci,
        Epoch {
            clock: ClockDomain::VehicleBoot,
            scale: TimeScale::Monotonic,
            nanos: 7,
        },
        Quat::IDENTITY,
        [0.0; 3],
    )
    .expect("valid");
    assert_eq!(base.then(&other_clock), Err(FrameError::ClockMismatch));

    let other_scale = FrameTransform::new(
        FrameId::Ecef,
        FrameId::Eci,
        Epoch {
            clock: ClockDomain::Simulation,
            scale: TimeScale::Gps,
            nanos: 7,
        },
        Quat::IDENTITY,
        [0.0; 3],
    )
    .expect("valid");
    assert_eq!(base.then(&other_scale), Err(FrameError::TimeScaleMismatch));
}

#[test]
fn invalid_rotations_and_translations_are_refused() {
    for q in [
        Quat {
            w: 0.0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        Quat {
            w: 2.0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
        Quat {
            w: f32::NAN,
            x: 0.0,
            y: 0.0,
            z: 0.0,
        },
    ] {
        assert_eq!(
            FrameTransform::new(FrameId::Body, FrameId::Ned, epoch(1), q, [0.0; 3]),
            Err(FrameError::InvalidTransform)
        );
    }
    assert_eq!(
        FrameTransform::new(
            FrameId::Body,
            FrameId::Ned,
            epoch(1),
            Quat::IDENTITY,
            [f64::NAN, 0.0, 0.0],
        ),
        Err(FrameError::InvalidTransform)
    );
    // Drift inside the tolerance renormalizes instead of failing.
    let drifted = Quat {
        w: 1.0 + ROTATION_NORM_TOLERANCE * 0.5,
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    let built = FrameTransform::new(FrameId::Body, FrameId::Ned, epoch(1), drifted, [0.0; 3])
        .expect("renormalizes");
    assert!((built.rotation().w - 1.0).abs() < 1e-6);
}

#[test]
fn every_supported_frame_pair_composes_through_a_chain() {
    use crate::frame::FrameId::*;
    // A hub-and-spoke chain touching all eight frames; composing along
    // it exercises every junction pairing the contract supports.
    let chain = [
        (Body, Installation),
        (Installation, Ned),
        (Ned, Ecef),
        (Ecef, Eci),
        (Eci, Lvlh),
        (Lvlh, Rtn),
        (Rtn, TargetRelative),
    ];
    let mut acc: Option<FrameTransform> = None;
    for (i, (from, to)) in chain.iter().enumerate() {
        let step = t(
            *from,
            *to,
            yaw_quat(0.1 * (i as f64 + 1.0)),
            [i as f64, 0.0, 1.0],
        );
        acc = Some(match acc {
            None => step,
            Some(prev) => prev.then(&step).expect("chain composes"),
        });
    }
    let full = acc.expect("chain built");
    assert_eq!(full.from_frame(), Body);
    assert_eq!(full.to_frame(), TargetRelative);
    let back = full.then(&full.inverse()).expect("returns");
    assert!(close(back.translation_m(), [0.0, 0.0, 0.0]));
}
