#![allow(clippy::expect_used, clippy::panic)]

//! Motion-lease reacquisition after a live profile switch: the runtime fences
//! the motion generation and gates every motion frame until the host regrants
//! on a fresh generation, so a remapped scheme never publishes on the released
//! generation.

use super::{SECOND_PROFILE, sample, session, with_default};
use crate::plan::LeaseAction;
use crate::profile::ProfileRuntime;
use crate::sample::{Mode, SessionState};

/// A session whose MOTION lease grant is set explicitly, for driving the
/// post-handover motion-authority reacquisition.
fn session_motion(mode: Mode, granted: bool, motion_granted: bool) -> SessionState {
    SessionState {
        motion_granted,
        ..session(mode, granted)
    }
}

/// A session at an explicit clock, for exercising the reacquisition retry.
fn session_at(now_ms: f64, mode: Mode, granted: bool, motion_granted: bool) -> SessionState {
    SessionState {
        now_ms,
        ..session_motion(mode, granted, motion_granted)
    }
}

#[test]
fn a_live_profile_switch_reacquires_motion_before_the_new_mapping_publishes() {
    let mut runtime = with_default();
    // Steady flight publishes motion frames on the held lease.
    let steady = runtime.evaluate(
        &sample(&[1.0, 0.0, 0.0, 0.0], &[]),
        &session(Mode::QuadPilot, true),
    );
    assert!(steady.motion.is_some(), "steady flight publishes motion");

    // Swap profiles with controls neutral: the candidate installs at once and
    // the handover releases the motion lease.
    runtime.activate(ProfileRuntime::compile(SECOND_PROFILE.as_bytes()).expect("compiles"));
    let install = runtime.evaluate(
        &sample(&[0.0, 0.0, 0.0, 0.0], &[]),
        &session(Mode::QuadPilot, true),
    );
    assert_eq!(runtime.activation_revision(), 2, "neutral controls install");
    assert_eq!(
        install.motion_lease,
        Some(LeaseAction::Release),
        "the handover releases the motion lease"
    );

    // Right after install the release has not landed yet: no request is sent,
    // and even a fully deflected stick produces NO motion frame. This is the
    // reviewer's repro — a live command on the released generation — gated out.
    let right_after = runtime.evaluate(
        &sample(&[1.0, 0.0, 0.0, 0.0], &[]),
        &session(Mode::QuadPilot, true),
    );
    assert_eq!(
        right_after.motion_lease, None,
        "no request until the release lands"
    );
    assert!(
        right_after.motion.is_none(),
        "no motion frame publishes on the stale generation"
    );

    // The host acknowledges the release (motion no longer granted): ONLY now is
    // the request emitted — release strictly precedes request — still gated.
    let requesting = runtime.evaluate(
        &sample(&[1.0, 0.0, 0.0, 0.0], &[]),
        &session_motion(Mode::QuadPilot, true, false),
    );
    assert_eq!(
        requesting.motion_lease,
        Some(LeaseAction::Request),
        "requests only after the release is reflected"
    );
    assert!(
        requesting.motion.is_none(),
        "motion stays gated awaiting the fresh grant"
    );

    // The host regrants on a fresh generation: motion resumes with the new
    // mapping and no further lease action fires.
    let resumed = runtime.evaluate(
        &sample(&[1.0, 0.0, 0.0, 0.0], &[]),
        &session_motion(Mode::QuadPilot, true, true),
    );
    assert!(resumed.motion.is_some(), "motion resumes once regranted");
    assert_eq!(
        resumed.motion_lease, None,
        "no lease action once held again"
    );
}

#[test]
fn a_dropped_motion_reacquire_request_is_retried() {
    let mut runtime = with_default();
    runtime.activate(ProfileRuntime::compile(SECOND_PROFILE.as_bytes()).expect("compiles"));
    // Install at t=1000 (motion still granted): the runtime enters Releasing.
    runtime.evaluate(
        &sample(&[0.0, 0.0, 0.0, 0.0], &[]),
        &session_at(1000.0, Mode::QuadPilot, true, true),
    );
    // Release lands at t=1010: the request is emitted and the clock recorded.
    let req = runtime.evaluate(
        &sample(&[0.0, 0.0, 0.0, 0.0], &[]),
        &session_at(1010.0, Mode::QuadPilot, true, false),
    );
    assert_eq!(req.motion_lease, Some(LeaseAction::Request));
    // 100 ms later with still no grant: inside the retry window, no re-request.
    let quiet = runtime.evaluate(
        &sample(&[0.0, 0.0, 0.0, 0.0], &[]),
        &session_at(1110.0, Mode::QuadPilot, true, false),
    );
    assert_eq!(
        quiet.motion_lease, None,
        "no re-request within the retry window"
    );
    // Past the retry window with the grant still missing: re-emit the request,
    // so a dropped lease write cannot wedge the reacquisition.
    let retry = runtime.evaluate(
        &sample(&[0.0, 0.0, 0.0, 0.0], &[]),
        &session_at(1300.0, Mode::QuadPilot, true, false),
    );
    assert_eq!(
        retry.motion_lease,
        Some(LeaseAction::Request),
        "a dropped reacquire request is retried"
    );
}
