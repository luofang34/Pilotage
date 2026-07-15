//! The kinematic-input validity gate: regressions for the invalid-input
//! blockers. A non-unit attitude, a mis-framed velocity or body rate, a
//! mis-provenanced kinematic input, an untrusted integrity (pose, velocity, or
//! body rate), a kinematic epoch off the pose clock or timed away from the
//! pose, and a wildly inaccurate velocity must each fail closed rather than
//! draw a conformal cue.
//!
//! Shares the parent module's fixtures via `use super::*`.

use super::*;

use pilotage_frames::FrameId;

use crate::availability::AvailabilityReason;
use crate::identity::IntegrityLevel;
use crate::{ConformalReason, ConformalState};

#[test]
fn a_zero_quaternion_attitude_is_non_conformal_and_draws_nothing() {
    // A zero (non-unit) quaternion cannot orient the projection. It must not yield
    // a drawable verdict — it is refused before interpolation.
    let zero = Quat {
        w: 0.0,
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
    let b = Bracket {
        older: sample(zero, [50.0, 0.0, 0.0], [0.0; 3], 1_000, 1),
        newer: sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 2_000, 2),
    };
    let fix = run(&b, capture(1_500), &sim());
    assert_eq!(
        fix.state,
        ConformalState::NonConformal(ConformalReason::AttitudeNotARotation)
    );
    assert!(fix.cues.is_none() && fix.error.is_none());
}

#[test]
fn unknown_integrity_is_unavailable_never_drawable() {
    // An `Unknown` integrity level fails closed through the derived availability:
    // it must never produce a drawable conformal cue.
    let mut older = sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 1_000, 1);
    older.attitude.stamp.integrity = IntegrityLevel::Unknown;
    older.position.stamp.integrity = IntegrityLevel::Unknown;
    let b = Bracket {
        older,
        newer: sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 2_000, 2),
    };
    let fix = run(&b, capture(1_500), &sim());
    assert!(
        matches!(
            fix.state,
            ConformalState::Unavailable(ConformalReason::Availability(_))
        ),
        "unknown integrity must be unavailable, got {:?}",
        fix.state
    );
    assert!(fix.cues.is_none(), "an unknown-integrity fix draws no cues");
}

#[test]
fn velocity_in_the_wrong_frame_is_non_conformal() {
    // The velocity must be NED; a body-frame velocity is refused rather than
    // projected as if it were NED.
    let mut older = sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 1_000, 1);
    older.velocity.frame = FrameId::Body;
    let b = Bracket {
        older,
        newer: sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 2_000, 2),
    };
    let fix = run(&b, capture(1_500), &sim());
    assert_eq!(
        fix.state,
        ConformalState::NonConformal(ConformalReason::KinematicFrame)
    );
    assert!(fix.cues.is_none());
}

#[test]
fn body_rate_in_the_wrong_frame_is_non_conformal() {
    // The body rate must be body-frame; an NED-tagged rate is refused.
    let mut older = sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 1_000, 1);
    older.body_rate.frame = FrameId::Ned;
    let b = Bracket {
        older,
        newer: sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 2_000, 2),
    };
    let fix = run(&b, capture(1_500), &sim());
    assert_eq!(
        fix.state,
        ConformalState::NonConformal(ConformalReason::KinematicFrame)
    );
    assert!(fix.cues.is_none());
}

#[test]
fn kinematics_from_a_different_snapshot_are_non_conformal() {
    // The velocity's provenance must be the pose's coherent snapshot; a velocity
    // stamped with a different snapshot is not one trustworthy fix.
    let mut older = sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 1_000, 1);
    older.velocity.meta.snapshot.id = 999;
    let b = Bracket {
        older,
        newer: sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 2_000, 2),
    };
    let fix = run(&b, capture(1_500), &sim());
    assert_eq!(
        fix.state,
        ConformalState::NonConformal(ConformalReason::KinematicProvenance)
    );
    assert!(fix.cues.is_none());
}

#[test]
fn an_unknown_integrity_velocity_alone_is_unavailable_never_drawable() {
    // The velocity steers the flight-path marker; a velocity the producer cannot
    // vouch for must fail the derived availability even when the pose is fully
    // trusted — nothing else about the sample is perturbed.
    let mut newer = sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 2_000, 2);
    newer.velocity.meta.integrity = IntegrityLevel::Unknown;
    let b = Bracket {
        older: sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 1_000, 1),
        newer,
    };
    let fix = run(&b, capture(1_500), &generous());
    assert_eq!(
        fix.state,
        ConformalState::Unavailable(ConformalReason::Availability(AvailabilityReason::Integrity))
    );
    assert!(fix.cues.is_none() && fix.error.is_none());
}

#[test]
fn an_unknown_integrity_body_rate_alone_is_unavailable_never_drawable() {
    // The body rate scales the timing sensitivity of the whole error budget; a
    // rate the producer cannot vouch for must fail the derived availability.
    let mut newer = sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 2_000, 2);
    newer.body_rate.meta.integrity = IntegrityLevel::Unknown;
    let b = Bracket {
        older: sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 1_000, 1),
        newer,
    };
    let fix = run(&b, capture(1_500), &generous());
    assert_eq!(
        fix.state,
        ConformalState::Unavailable(ConformalReason::Availability(AvailabilityReason::Integrity))
    );
    assert!(fix.cues.is_none() && fix.error.is_none());
}

#[test]
fn a_monitored_velocity_degrades_the_scene_to_limited() {
    // A monitored (reduced-integrity) velocity degrades rather than fails: the
    // cues stay drawable but must carry the reduced-confidence mark.
    let mut b = steady(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3]);
    b.newer.velocity.meta.integrity = IntegrityLevel::Monitored;
    let fix = run(&b, capture(1_500), &generous());
    assert_eq!(
        fix.state,
        ConformalState::Limited(ConformalReason::Availability(AvailabilityReason::Integrity))
    );
    assert!(fix.cues.expect("a limited fix draws cues").limited);
}

#[test]
fn a_velocity_timed_seconds_from_the_pose_is_non_conformal() {
    // A velocity tagged nine seconds away from the pose epoch is not the state
    // at the pose's instant, however coherent its snapshot identity is — the
    // spread is charged as co-timing skew and exceeds the policy limit.
    let mut newer = sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 2_000, 2);
    newer.velocity.epoch = epoch(9_000_002_000);
    newer.velocity.meta.acquired_at = epoch(9_000_002_000);
    let b = Bracket {
        older: sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 1_000, 1),
        newer,
    };
    let fix = run(&b, capture(1_500), &generous());
    assert_eq!(
        fix.state,
        ConformalState::NonConformal(ConformalReason::ExcessiveSkew)
    );
    assert!(fix.cues.is_none());
}

#[test]
fn a_velocity_value_epoch_alone_timed_from_the_pose_is_non_conformal() {
    // The value epoch (the instant the velocity is valid at) participates in the
    // skew on its own, even when the acquisition stamp stays co-timed.
    let mut older = sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 1_000, 1);
    older.velocity.epoch = epoch(9_000_001_000);
    let b = Bracket {
        older,
        newer: sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 2_000, 2),
    };
    let fix = run(&b, capture(1_500), &generous());
    assert_eq!(
        fix.state,
        ConformalState::NonConformal(ConformalReason::ExcessiveSkew)
    );
    assert!(fix.cues.is_none());
}

#[test]
fn a_body_rate_acquired_seconds_from_the_pose_is_non_conformal() {
    // The body rate's acquisition epoch participates in the skew exactly like
    // the velocity's.
    let mut newer = sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 2_000, 2);
    newer.body_rate.meta.acquired_at = epoch(9_000_002_000);
    let b = Bracket {
        older: sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 1_000, 1),
        newer,
    };
    let fix = run(&b, capture(1_500), &generous());
    assert_eq!(
        fix.state,
        ConformalState::NonConformal(ConformalReason::ExcessiveSkew)
    );
    assert!(fix.cues.is_none());
}

#[test]
fn a_kinematic_value_epoch_on_a_different_clock_is_non_conformal() {
    // A velocity value epoch on a different clock cannot be compared to the pose
    // epoch at all, so it is refused as clock-incoherent rather than having its
    // nanoseconds read as if they were on the pose clock.
    let mut older = sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 1_000, 1);
    older.velocity.epoch.clock = ClockDomain::Gnss;
    let b = Bracket {
        older,
        newer: sample(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3], 2_000, 2),
    };
    let fix = run(&b, capture(1_500), &generous());
    assert_eq!(
        fix.state,
        ConformalState::NonConformal(ConformalReason::ClockIncoherent)
    );
    assert!(fix.cues.is_none());
}

#[test]
fn a_small_kinematic_skew_is_charged_to_the_budget_not_refused() {
    // A 5 ms velocity skew is within the 10 ms policy limit: the fix stays
    // drawable, and the skew lands in the latency term of the error budget
    // rather than being silently bridged.
    let nominal = run(
        &steady(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3]),
        capture(1_500),
        &generous(),
    );
    let mut skewed = steady(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3]);
    skewed.newer.velocity.epoch = epoch(5_002_000);
    skewed.newer.velocity.meta.acquired_at = epoch(5_002_000);
    let fix = run(&skewed, capture(1_500), &generous());
    assert!(fix.state.draws_cues(), "a within-limit skew stays drawable");
    let (e0, e1) = (
        nominal.error.expect("ran"),
        fix.error.expect("interpolation ran"),
    );
    assert!(
        e1.latency_rad > e0.latency_rad && e1.total_rad > e0.total_rad,
        "the kinematic skew is charged into the latency term"
    );
}

#[test]
fn velocity_error_is_a_named_budget_term_that_scales_with_the_accuracy() {
    // A nominal fix carries a finite, positive, named velocity-error term.
    let nominal = run(
        &steady(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3]),
        capture(1_500),
        &sim(),
    );
    let e0 = nominal.error.expect("interpolation ran");
    assert!(
        e0.velocity_rad.is_finite() && e0.velocity_rad > 0.0,
        "velocity error is a named, present term"
    );

    // A tenfold-worse velocity 1-sigma over the same timing yields ~tenfold the
    // velocity term and a larger total — the term genuinely consumes the accuracy.
    let mut worse = steady(Quat::IDENTITY, [50.0, 0.0, 0.0], [0.0; 3]);
    worse.older.velocity_quality.sigma_mmps *= 10;
    worse.newer.velocity_quality.sigma_mmps *= 10;
    let e1 = run(&worse, capture(1_500), &sim())
        .error
        .expect("interpolation ran");
    assert!(
        e1.velocity_rad > 5.0 * e0.velocity_rad && e1.total_rad > e0.total_rad,
        "the velocity term scales with the velocity accuracy and moves the total"
    );
}

#[test]
fn a_wildly_inaccurate_velocity_alone_removes_the_cues() {
    // Low speed keeps the timing sensitivity small, so a 44 ms extrapolation is
    // within both the limit and the budget when the velocity is accurate...
    let b = Bracket {
        older: sample(Quat::IDENTITY, [5.0, 0.0, 0.0], [0.0; 3], 1_000_000, 1),
        newer: sample(Quat::IDENTITY, [5.0, 0.0, 0.0], [0.0; 3], 2_000_000, 2),
    };
    assert!(
        run(&b, capture(46_000_000), &sim()).state.draws_cues(),
        "an accurate slow state is drawable at 44 ms extrapolation"
    );

    // ...but a wildly inaccurate velocity (40 m/s 1-sigma), propagated over that
    // extrapolation, pushes the parallax bound past the budget and removes every
    // cue — the velocity accuracy alone decides, nothing else changed.
    let mut broken = b;
    broken.older.velocity_quality.sigma_mmps = 40_000;
    broken.newer.velocity_quality.sigma_mmps = 40_000;
    let fix = run(&broken, capture(46_000_000), &sim());
    assert_eq!(
        fix.state,
        ConformalState::NonConformal(ConformalReason::ErrorBudgetExceeded)
    );
    assert!(fix.cues.is_none());
    assert!(
        fix.error.expect("ran").velocity_rad > sim().valid_error_rad(),
        "the velocity term dominates the budget"
    );
}
