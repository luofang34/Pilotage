#![allow(clippy::expect_used, clippy::panic)]

//! Motion-lease reacquisition after a live profile switch. The runtime fences
//! the motion generation, gates live output through the whole handshake, and
//! recovers ONLY once the operator's controls read neutral AND the host
//! confirms it cleared the vehicle's link-loss latch — so a remapped scheme
//! never publishes on the released generation and never resumes on the hope a
//! best-effort datagram arrived. A denied reacquire is terminal.

use super::{SECOND_PROFILE, sample, session, with_default};
use crate::plan::LeaseAction;
use crate::profile::ProfileRuntime;
use crate::sample::{Mode, RawSample, SessionState};

/// A session whose MOTION lease grant is set explicitly.
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

/// A session whose motion reacquire was DENIED by the host.
fn session_denied(now_ms: f64, mode: Mode) -> SessionState {
    SessionState {
        now_ms,
        motion_denied: true,
        ..session_motion(mode, true, false)
    }
}

/// A session mid-recovery: the motion lease is granted, with the host's
/// link-loss-cleared confirmation (`motion_recovered`) set explicitly.
fn recovering(motion_granted: bool, motion_recovered: bool) -> SessionState {
    SessionState {
        motion_granted,
        motion_recovered,
        ..session(Mode::QuadPilot, true)
    }
}

fn deflected() -> RawSample {
    sample(&[1.0, 1.0, 1.0, 1.0], &[])
}

fn neutral() -> RawSample {
    sample(&[0.0, 0.0, 0.0, 0.0], &[])
}

/// Whether the plan carries a motion frame whose every axis reads zero.
fn is_neutral_motion(plan: &crate::plan::ControlPlan) -> bool {
    plan.motion
        .as_ref()
        .is_some_and(|frame| frame.axes().iter().all(|(_, value)| *value == 0.0))
}

#[test]
fn a_live_switch_recovers_only_when_the_host_confirms() {
    let mut runtime = with_default();
    // Steady flight publishes the live stick.
    assert!(
        runtime
            .evaluate(&deflected(), &session(Mode::QuadPilot, true))
            .motion
            .is_some(),
        "steady flight publishes live motion"
    );

    // Handover installs with controls neutral, releasing the motion lease.
    runtime.activate(ProfileRuntime::compile(SECOND_PROFILE.as_bytes()).expect("compiles"));
    let install = runtime.evaluate(&neutral(), &session(Mode::QuadPilot, true));
    assert_eq!(runtime.activation_revision(), 2, "neutral controls install");
    assert_eq!(install.motion_lease, Some(LeaseAction::Release));

    // Release lands → request; live output is gated throughout.
    let req = runtime.evaluate(&deflected(), &session_motion(Mode::QuadPilot, true, false));
    assert_eq!(req.motion_lease, Some(LeaseAction::Request));
    assert!(req.motion.is_none(), "gated while reacquiring");

    // Regranted, but the operator is STILL deflected: motion stays fully gated
    // (no live command, no neutral activation while deflected).
    assert!(
        runtime
            .evaluate(&deflected(), &recovering(true, false))
            .motion
            .is_none(),
        "gated while the operator is deflected"
    );

    // The operator centers but the host has NOT confirmed recovery: the runtime
    // retransmits neutral activation frames EVERY tick — indefinitely — and
    // never resumes live on the mere hope a datagram arrived.
    for i in 0..6 {
        let plan = runtime.evaluate(&neutral(), &recovering(true, false));
        assert!(
            is_neutral_motion(&plan),
            "neutral activation retransmit {i}"
        );
    }
    // Even a later deflection, still unconfirmed, does NOT go live.
    assert!(
        runtime
            .evaluate(&deflected(), &recovering(true, false))
            .motion
            .is_none(),
        "unconfirmed recovery never resumes live, however many neutral frames were sent"
    );

    // The host CONFIRMS it cleared the vehicle's link-loss latch: a final
    // neutral this tick, then live resumes with the operator already neutral.
    let confirmed = runtime.evaluate(&neutral(), &recovering(true, true));
    assert!(
        is_neutral_motion(&confirmed),
        "a final neutral on confirmation"
    );
    let resumed = runtime.evaluate(&deflected(), &recovering(true, true));
    assert!(
        resumed.motion.is_some(),
        "live resumes once the host confirms"
    );
    assert!(
        !is_neutral_motion(&resumed),
        "the resumed frame is the live deflected stick"
    );
}

#[test]
fn a_denied_motion_reacquire_is_terminal() {
    let mut runtime = with_default();
    runtime.activate(ProfileRuntime::compile(SECOND_PROFILE.as_bytes()).expect("compiles"));
    // Install (Releasing) at t=1000, then the release lands → request at t=1010.
    runtime.evaluate(&neutral(), &session_at(1000.0, Mode::QuadPilot, true, true));
    let req = runtime.evaluate(
        &neutral(),
        &session_at(1010.0, Mode::QuadPilot, true, false),
    );
    assert_eq!(req.motion_lease, Some(LeaseAction::Request));

    // The host DENIES the reacquire: terminal — no re-request, motion gated.
    let denied = runtime.evaluate(&deflected(), &session_denied(1020.0, Mode::QuadPilot));
    assert_eq!(denied.motion_lease, None, "a denial stops requesting");
    assert!(denied.motion.is_none(), "denied motion is gated");

    // Well past the retry window, still no request and still gated (terminal).
    let later = runtime.evaluate(&deflected(), &session_denied(3000.0, Mode::QuadPilot));
    assert_eq!(
        later.motion_lease, None,
        "denial is terminal — never retries"
    );
    assert!(later.motion.is_none());
}

#[test]
fn a_dropped_motion_reacquire_request_is_retried() {
    let mut runtime = with_default();
    runtime.activate(ProfileRuntime::compile(SECOND_PROFILE.as_bytes()).expect("compiles"));
    // Install at t=1000 (motion still granted): the runtime enters Releasing.
    runtime.evaluate(&neutral(), &session_at(1000.0, Mode::QuadPilot, true, true));
    // Release lands at t=1010: the request is emitted and the clock recorded.
    let req = runtime.evaluate(
        &neutral(),
        &session_at(1010.0, Mode::QuadPilot, true, false),
    );
    assert_eq!(req.motion_lease, Some(LeaseAction::Request));
    // 100 ms later with still no grant: inside the retry window, no re-request.
    let quiet = runtime.evaluate(
        &neutral(),
        &session_at(1110.0, Mode::QuadPilot, true, false),
    );
    assert_eq!(
        quiet.motion_lease, None,
        "no re-request within the retry window"
    );
    // Past the retry window with the grant still missing: re-emit the request,
    // so a dropped lease write cannot wedge the reacquisition.
    let retry = runtime.evaluate(
        &neutral(),
        &session_at(1300.0, Mode::QuadPilot, true, false),
    );
    assert_eq!(
        retry.motion_lease,
        Some(LeaseAction::Request),
        "a dropped reacquire request is retried"
    );
}
