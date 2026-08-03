//! CDI realism: the display-facing cross-track deviation is checked
//! closed-loop, against a vehicle that is actually off course.
//!
//! A deviation readout can be wrong in three ways a static geometry test
//! never reaches: it can carry the wrong sign, it can carry a magnitude
//! that does not match where the vehicle is, and it can fail to move as
//! the vehicle corrects. Each scenario here displaces the vehicle from
//! the course it is tracking and then flies the engine's own commands
//! back onto it, so the assertions are about a needle that deflects the
//! right way and comes home.
//!
//! The assertions sit at the mission level rather than on the broadcast
//! wire: `hosts/session-host/tests/mission_reference.rs` already pins the
//! publication path — that guidance reaches telemetry stamped, named
//! after a fixture fix, and framed as `NavGuidanceState` — and it flies a
//! planar skiff that cannot traverse geodetic waypoints, so it cannot
//! produce a real deviation to judge. The geometry is decided in
//! `MissionEngine::nav_guidance`, and that is where a wrong sign or a
//! diverging correction is observable.

#![allow(clippy::expect_used, clippy::panic)]

// An explicit path keeps the rig out of the integration-test target list:
// every `tests/*.rs` is compiled as its own binary, so a sibling module
// file would become a second, empty test target.
#[path = "cdi_realism/closed_loop.rs"]
mod closed_loop;

use closed_loop::{
    CORNER_IDENTS, Rig, corner_engine, fixture_engine, right_of_course_ned, try_engine_over_offsets,
};
use navigate_fpl::{PlanActivationError, SequenceReason};
use pilotage_mission::{MissionBuildError, MissionEvent, NavQuality};

/// Lateral disturbance a displacement scenario applies, m/s. Above the
/// guidance correction ceiling, so the deviation grows while the push is
/// on; small enough per 50 ms step that the fusion innovation gate admits
/// every fix, making the displacement an observed one rather than a
/// teleport the filter would reject.
const PUSH_MPS: f64 = 6.0;

/// Lateral displacement a scenario builds before releasing the push,
/// meters.
const DISPLACEMENT_M: f64 = 40.0;

/// Deviation under which a displaced vehicle counts as recaptured,
/// meters.
const RECAPTURED_M: f64 = 5.0;

/// Deviation under which a corner transient counts as decayed, meters.
const CORNER_DECAYED_M: f64 = 10.0;

/// Per-step growth tolerated inside a converging trace, meters. The
/// filter's estimate lags a pushed truth and catches up once the push
/// releases, so the reported deviation can tick up before it turns over.
const JITTER_M: f64 = 0.5;

/// How a deviation decayed.
struct Convergence {
    /// Deviation magnitude the correction started from, meters.
    start_m: f64,
    /// Step at which the magnitude first reached half the start.
    half_at: usize,
    /// Step at which the magnitude first fell under the target.
    settled_at: usize,
    /// Largest single-step increase in magnitude anywhere in the trace.
    worst_rise_m: f64,
}

/// Flies the engine's own commands until the deviation falls under
/// `target_m`, reporting the trace.
///
/// Fails when the budget runs out — a correction that never arrives is
/// the defect this file exists to catch — when the plan sequences to
/// another leg mid-correction, since a deviation measured against a
/// different track is a different number, or when the desired track moves
/// while the vehicle corrects toward it.
fn converge_on_leg(
    rig: &mut Rig,
    leg_index: u32,
    course_rad: f64,
    target_m: f64,
    budget: usize,
) -> Convergence {
    let start_m = rig
        .expect_deviation("correcting a lateral displacement")
        .abs();
    let mut previous_m = start_m;
    let mut half_at = None;
    let mut worst_rise_m = 0.0_f64;
    for taken in 1..=budget {
        rig.step();
        let guidance = rig.expect_guidance("correcting a lateral displacement");
        assert_eq!(
            guidance.leg_index, leg_index,
            "leg sequenced at step {taken}: the correction ran out of leg to fly"
        );
        assert!(
            (guidance.course_rad - course_rad).abs() < 1e-9,
            "the desired track moved to {} while correcting toward {course_rad}",
            guidance.course_rad
        );
        let deviation_m = guidance
            .lateral_deviation_m
            .expect("a fixed leg reports cross-track deviation")
            .abs();
        worst_rise_m = worst_rise_m.max(deviation_m - previous_m);
        previous_m = deviation_m;
        if half_at.is_none() && deviation_m <= start_m / 2.0 {
            half_at = Some(taken);
        }
        if deviation_m < target_m {
            return Convergence {
                start_m,
                half_at: half_at.unwrap_or(taken),
                settled_at: taken,
                worst_rise_m,
            };
        }
    }
    panic!("{start_m} m of deviation never fell under {target_m} m in {budget} steps");
}

/// Displaces the vehicle `DISPLACEMENT_M` to one side of the leg it is
/// tracking and flies it back. `side` is `1.0` for right of course,
/// `-1.0` for left.
fn displace_and_recapture(side: f64) {
    let mut rig = Rig::new(corner_engine());
    // The corner route's direct-to leg runs along the extension of its
    // first fixed leg, so the correction starts from a settled course
    // rather than from a transition transient of its own.
    assert!(
        rig.fly_until(5_000, |g| g.leg_index == 1
            && g.lateral_deviation_m.is_some_and(|d| d.abs() < 1.0))
            .is_some(),
        "the first fixed leg is tracked within the step budget"
    );
    let tracking = rig.expect_guidance("tracking the first fixed leg");
    assert_eq!(tracking.to_ident, CORNER_IDENTS[1]);
    assert_eq!(tracking.quality, NavQuality::Good);

    rig.disturbance_ned = right_of_course_ned(tracking.course_rad, PUSH_MPS * side);
    assert!(
        rig.fly_until(1_000, |g| g
            .lateral_deviation_m
            .is_some_and(|d| d.abs() >= DISPLACEMENT_M))
            .is_some(),
        "the disturbance builds {DISPLACEMENT_M} m of deviation within the step budget"
    );
    rig.disturbance_ned = [0.0; 2];

    let displaced_m = rig.expect_deviation("displaced from the tracked leg");
    assert_eq!(
        displaced_m.signum(),
        side,
        "a vehicle pushed {} of course must read {} deviation, got {displaced_m}",
        if side > 0.0 { "right" } else { "left" },
        if side > 0.0 { "positive" } else { "negative" },
    );
    assert!(
        (DISPLACEMENT_M..DISPLACEMENT_M + 2.0).contains(&displaced_m.abs()),
        "the readout must match the displacement it was flown to, got {displaced_m}"
    );
    assert_eq!(
        rig.engine.counters().fusion_rejected,
        0,
        "the displacement was observed, not rejected as an outlier"
    );

    let trace = converge_on_leg(&mut rig, 1, tracking.course_rad, RECAPTURED_M, 800);
    assert!(
        trace.half_at <= 300,
        "{} m halved only after {} steps",
        trace.start_m,
        trace.half_at
    );
    assert!(
        trace.settled_at <= 600,
        "recapture took {} steps",
        trace.settled_at
    );
    assert!(
        trace.worst_rise_m < JITTER_M,
        "the deviation grew {} m mid-correction",
        trace.worst_rise_m
    );
}

#[test]
fn cdi_realism_displaced_capture_converges_from_right_of_course() {
    displace_and_recapture(1.0);
}

#[test]
fn cdi_realism_displaced_capture_converges_from_left_of_course() {
    displace_and_recapture(-1.0);
}

/// A 90° dogleg: the desired track turns a quarter circle at the corner,
/// and the deviation the corner geometry hands the new leg decays.
#[test]
fn cdi_realism_dogleg_turns_the_course_a_quarter_circle() {
    let mut rig = Rig::new(corner_engine());
    assert!(
        rig.fly_until(5_000, |g| g.leg_index == 1).is_some(),
        "the inbound leg is reached within the step budget"
    );
    // The last guidance published before the corner sequences is what the
    // display was showing on the way in; the turn is the difference
    // between that and the first guidance published after.
    let mut inbound = rig.expect_guidance("flying the inbound leg");
    let mut advance = None;
    for _ in 0..10_000 {
        advance = rig.step().events.iter().find_map(|event| match event {
            MissionEvent::LegAdvanced {
                to_index: 2,
                reason,
            } => Some(*reason),
            _ => None,
        });
        if advance.is_some() {
            break;
        }
        inbound = rig.expect_guidance("approaching the corner");
    }
    let reason = advance.expect("the corner sequences within the step budget");
    let outbound = rig.expect_guidance("flying the leg past the corner");
    assert_eq!(inbound.to_ident, CORNER_IDENTS[1]);
    assert_eq!(outbound.to_ident, CORNER_IDENTS[2]);

    let turn_rad = wrap_to_pi(outbound.course_rad - inbound.course_rad);
    assert!(
        (turn_rad - core::f64::consts::FRAC_PI_2).abs() < 0.01,
        "the corner turns {turn_rad} rad, not a quarter circle right"
    );

    // The identity below holds only because the corner is crossed by
    // capture radius rather than anticipated: at the instant the fix
    // sequences the vehicle is still short of it, and for a square corner
    // the distance still to run becomes cross-track deviation from the new
    // leg, one for one. At the mission cruise speed the fly-by turn radius
    // is on the order of a meter, so anticipation never reaches the
    // capture radius.
    assert_eq!(
        reason,
        SequenceReason::Overflown,
        "an anticipated corner would sequence before the run-in this asserts on"
    );
    let short_of_corner_m = inbound.distance_to_waypoint_m;
    let transient_m = outbound
        .lateral_deviation_m
        .expect("the leg past the corner has an origin fix");
    assert!(
        transient_m > 0.0,
        "short of a right turn is right of the new course, got {transient_m}"
    );
    assert!(
        (transient_m - short_of_corner_m).abs() < 2.0,
        "a square corner hands the new leg its own run-in: {short_of_corner_m} m short read as {transient_m} m off"
    );

    let trace = converge_on_leg(&mut rig, 2, outbound.course_rad, CORNER_DECAYED_M, 2_000);
    assert!(
        trace.settled_at <= 1_500,
        "the corner transient took {} steps to decay",
        trace.settled_at
    );
    assert!(
        trace.worst_rise_m < JITTER_M,
        "the corner transient grew {} m before decaying",
        trace.worst_rise_m
    );
}

/// The shipped demo route's first transition, on the geometry an operator
/// actually flies: its anchor lies well right of the `DEMOA`→`DEMOB`
/// track, so the display must show most of a capture radius of right
/// deviation the moment the first fix sequences — and then null it.
#[test]
fn cdi_realism_demo_fixture_transition_deflects_right_and_recovers() {
    let mut rig = Rig::new(fixture_engine());
    assert!(
        rig.fly_until(5_000, |g| g.leg_index == 1).is_some(),
        "the demo route sequences its first fix within the step budget"
    );
    let entry = rig.expect_guidance("entering the demo route's first fixed leg");
    assert_eq!(entry.from_ident.as_deref(), Some("DEMOA"));
    assert_eq!(entry.to_ident, "DEMOB");
    let transient_m = entry
        .lateral_deviation_m
        .expect("a fixed leg reports cross-track deviation");
    assert!(
        (90.0..100.0).contains(&transient_m),
        "the demo transition sits nearly a capture radius right of the new track, got {transient_m}"
    );

    let trace = converge_on_leg(&mut rig, 1, entry.course_rad, CORNER_DECAYED_M, 2_000);
    assert!(
        trace.settled_at <= 1_200,
        "the demo transient took {} steps to decay",
        trace.settled_at
    );
    assert!(
        trace.worst_rise_m < JITTER_M,
        "the demo transient grew {} m before decaying",
        trace.worst_rise_m
    );
}

/// Folds a course difference into `[-π, π]`: a quarter turn right is
/// `+π/2` whichever side of north the two courses fall.
fn wrap_to_pi(angle_rad: f64) -> f64 {
    let wrapped = (angle_rad + core::f64::consts::PI).rem_euclid(core::f64::consts::TAU);
    wrapped - core::f64::consts::PI
}

/// A route whose final leg cannot clear the capture radius refuses to
/// build as a typed activation error, and the rendered refusal names the
/// offending leg — what an operator reading the single host log line
/// actually sees. Guards the boundary where the sequencer's activation
/// check surfaces out of `MissionEngine::new`.
#[test]
fn activation_refuses_a_leg_no_longer_than_the_capture_radius() {
    let idents = ["TIGHA", "TIGHB", "TIGHC"];
    // 500 m first leg, then a 90 m closer — inside the 100 m capture
    // radius the mission defaults configure.
    let offsets = [(0.0, 400.0), (0.0, 900.0), (-90.0, 900.0)];
    let error = try_engine_over_offsets(&idents, &offsets, "TIGHA TIGHB TIGHC")
        .map(|_| ())
        .expect_err("a 90 m leg inside the 100 m capture radius must refuse to build");
    assert!(
        matches!(
            error,
            MissionBuildError::PlanActivation(PlanActivationError::CaptureRadiusExceedsLeg { .. })
        ),
        "expected a typed capture-radius refusal, got {error:?}"
    );
    let rendered = error.to_string();
    assert!(
        rendered.contains("TIGHC") && rendered.contains("capture radius"),
        "the rendered refusal must name the offending leg: {rendered}"
    );
}
