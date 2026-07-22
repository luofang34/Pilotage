#![allow(clippy::expect_used, clippy::panic)]

//! Motion-lease reacquisition after a live scope-member transfer. The runtime fences
//! the motion generation, gates live output through the whole handshake, and
//! recovers ONLY once the operator's controls read neutral AND the host
//! confirms it cleared the vehicle's link-loss latch — so a remapped scheme
//! never publishes on the released generation and never resumes on the hope a
//! best-effort datagram arrived. A denied reacquire is terminal.

use super::{SECOND_PROFILE, grant, sample, session, session_gen, with_default};
use crate::authority::{AuthorityEvent, AuthorityScope};
use crate::plan::LeaseAction;
use crate::profile::ProfileRuntime;
use crate::sample::{Mode, RawSample, SessionState};

/// A session whose MOTION lease grant is set explicitly.
fn session_motion(mode: Mode, granted: bool, _motion_granted: bool) -> SessionState {
    session(mode, granted)
}

/// A session at an explicit clock, for exercising the reacquisition retry.
fn session_at(now_ms: f64, mode: Mode, granted: bool, motion_granted: bool) -> SessionState {
    let mut state = session_motion(mode, granted, motion_granted);
    state.now_ms = now_ms;
    state
}

/// A session whose motion reacquire was DENIED by the host.
fn session_denied(now_ms: f64, mode: Mode) -> SessionState {
    session_at(now_ms, mode, true, false)
}

/// A session mid-recovery: the motion lease is granted, with the host's
/// link-loss-cleared confirmation (`motion_recovered`) set explicitly.
fn recovering(_motion_granted: bool, _motion_recovered: bool) -> SessionState {
    session(Mode::QuadPilot, true)
}

fn release_motion(runtime: &mut super::ControlRuntime, generation: u64) {
    runtime.authority_event(
        AuthorityScope::Motion,
        AuthorityEvent::LeaseReleased { generation },
    );
}

fn recover_motion(runtime: &mut super::ControlRuntime, generation: u64) {
    runtime.authority_event(
        AuthorityScope::Motion,
        AuthorityEvent::LinkLossCleared { generation },
    );
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
fn a_same_scope_activation_retains_authority_and_requires_installed_neutral() {
    let mut runtime = with_default();
    let candidate = ProfileRuntime::compile(SECOND_PROFILE.as_bytes()).expect("compiles");
    let activation = runtime.activate(candidate);
    assert!(!activation.release_motion_lease);
    assert!(!activation.release_gimbal_lease);

    let pending = runtime.evaluate(&deflected(), &session(Mode::QuadPilot, true));
    assert!(is_neutral_motion(&pending));
    assert_eq!(pending.motion_lease, None);
    assert_eq!(pending.lease, None);
    assert_eq!(runtime.activation_revision(), 1);

    let installed = runtime.evaluate(&neutral(), &session(Mode::QuadPilot, true));
    assert!(
        installed.motion.is_none() && installed.gimbal.is_none(),
        "the install tick emits no frames: the re-announcement must reach \
         the host before any datagram carries the advanced revision"
    );
    assert_eq!(runtime.activation_revision(), 2);
    assert_eq!(installed.motion_lease, None);
    assert_eq!(installed.lease, None);

    let gated = runtime.evaluate(&deflected(), &session(Mode::QuadPilot, true));
    assert!(
        gated.motion.is_none(),
        "the installed mapping must read neutral"
    );
    assert_eq!(gated.motion_lease, None, "authority remains held");

    let proof = runtime.evaluate(&neutral(), &session(Mode::QuadPilot, true));
    assert!(is_neutral_motion(&proof), "neutral proof reaches the host");
    let resumed = runtime.evaluate(&deflected(), &session(Mode::QuadPilot, true));
    assert!(resumed.motion.is_some());
    assert!(!is_neutral_motion(&resumed));
}

#[test]
fn an_unrecovered_fresh_generation_reenters_neutral_activation_recovery() {
    let mut runtime = with_default();
    runtime.evaluate(&neutral(), &session(Mode::QuadPilot, true));
    release_motion(&mut runtime, 1);
    grant(&mut runtime, AuthorityScope::Motion, 2);
    runtime.begin_control_run();
    let unrecovered = session_gen(2, Mode::QuadPilot, true);

    // The regrant after an input-loss suspension arrives on a fresh
    // generation with the link-loss clearance still outstanding: a held
    // deflection stays gated, never published.
    let held = runtime.evaluate(&deflected(), &unrecovered);
    assert!(held.motion.is_none(), "a held deflection cannot publish");
    let centered = runtime.evaluate(&neutral(), &unrecovered);
    assert!(
        is_neutral_motion(&centered),
        "centered controls retransmit the neutral activation"
    );
    let deflected_again = runtime.evaluate(&deflected(), &unrecovered);
    assert!(
        deflected_again.motion.is_none(),
        "re-deflecting mid-recovery gates again"
    );

    // Host confirmation: one final neutral, then live publishes.
    recover_motion(&mut runtime, 2);
    let confirmed = runtime.evaluate(&deflected(), &session_gen(2, Mode::QuadPilot, true));
    assert!(
        is_neutral_motion(&confirmed),
        "confirmation completes with one final neutral"
    );
    let live = runtime.evaluate(&deflected(), &session_gen(2, Mode::QuadPilot, true));
    assert!(
        live.motion.is_some() && !is_neutral_motion(&live),
        "live output resumes only after the host confirmed"
    );
}

#[test]
fn a_button_pressed_while_suspended_fires_no_edge_on_the_fresh_generation() {
    let mut runtime = with_default();
    runtime.evaluate(&neutral(), &session(Mode::QuadPilot, true));
    release_motion(&mut runtime, 1);
    grant(&mut runtime, AuthorityScope::Motion, 2);
    runtime.begin_control_run();

    // The arm button (9) went down while control was suspended (no ticks).
    // The fresh generation's first tick seeds the baseline from the held
    // state, so the press cannot fire; recovery gates it regardless.
    let arm_held = sample(&[0.0; 4], &[9]);
    let unrecovered = session_gen(2, Mode::QuadPilot, true);
    let primed = runtime.evaluate(&arm_held, &unrecovered);
    assert!(
        !primed.arm,
        "a press held across the suspension fires no edge"
    );
    assert!(
        !primed.arm_suppressed,
        "a seeded baseline is not a suppression"
    );

    // Released and pressed again after the host confirmed: a genuine edge.
    runtime.evaluate(&neutral(), &unrecovered);
    recover_motion(&mut runtime, 2);
    runtime.evaluate(&neutral(), &session_gen(2, Mode::QuadPilot, true));
    runtime.evaluate(&neutral(), &session_gen(2, Mode::QuadPilot, true));
    let fresh_press = runtime.evaluate(&arm_held, &session_gen(2, Mode::QuadPilot, true));
    assert!(fresh_press.arm, "a fresh press after recovery arms");
}

#[test]
fn a_fresh_session_voids_a_pending_transfer_release() {
    let mut runtime = with_default();
    // Steady flight on generation 1, then a scope transfer opens — but the
    // transport dies before its release lands. Bootstrap re-leases the boot
    // scope, so the transfer is moot: the reconnect must not release the
    // fresh session's lease on its behalf.
    runtime.evaluate(&neutral(), &session(Mode::QuadPilot, true));
    assert!(runtime.reactivate());
    runtime.begin_session();
    grant(&mut runtime, AuthorityScope::Motion, 2);
    grant(&mut runtime, AuthorityScope::Gimbal, 2);
    runtime.begin_control_run();

    let installed = runtime.evaluate(&neutral(), &session_gen(2, Mode::QuadPilot, true));
    assert_eq!(
        installed.motion_lease, None,
        "no release rides the fresh session"
    );
    assert_eq!(
        runtime.activation_revision(),
        2,
        "the pending install completes as a mapping change"
    );

    let proof = runtime.evaluate(&neutral(), &session_gen(2, Mode::QuadPilot, true));
    assert!(
        is_neutral_motion(&proof),
        "one conservative neutral follows the install"
    );
    let live = runtime.evaluate(&deflected(), &session_gen(2, Mode::QuadPilot, true));
    assert!(
        live.motion.is_some() && !is_neutral_motion(&live),
        "live resumes on the held bootstrap authority"
    );
    assert_eq!(live.motion_lease, None, "the lease was never cycled");
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
    assert!(runtime.reactivate());
    let install = runtime.evaluate(&neutral(), &session(Mode::QuadPilot, true));
    assert_eq!(runtime.activation_revision(), 2, "neutral controls install");
    assert_eq!(install.motion_lease, Some(LeaseAction::Release));

    // Release lands → request; live output is gated throughout.
    release_motion(&mut runtime, 1);
    let req = runtime.evaluate(&deflected(), &session_motion(Mode::QuadPilot, true, false));
    assert_eq!(req.motion_lease, Some(LeaseAction::Request));
    assert!(req.motion.is_none(), "gated while reacquiring");

    // Regranted, but the operator is STILL deflected: motion stays fully gated
    // (no live command, no neutral activation while deflected).
    grant(&mut runtime, AuthorityScope::Motion, 2);
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
    recover_motion(&mut runtime, 2);
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
    assert!(runtime.reactivate());
    // Install (Releasing) at t=1000, then the release lands → request at t=1010.
    runtime.evaluate(&neutral(), &session_at(1000.0, Mode::QuadPilot, true, true));
    release_motion(&mut runtime, 1);
    let req = runtime.evaluate(
        &neutral(),
        &session_at(1010.0, Mode::QuadPilot, true, false),
    );
    assert_eq!(req.motion_lease, Some(LeaseAction::Request));

    // The host DENIES the reacquire: terminal — no re-request, motion gated.
    runtime.authority_event(AuthorityScope::Motion, AuthorityEvent::LeaseDenied);
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
    assert!(runtime.reactivate());
    // Install at t=1000 (motion still granted): the runtime enters Releasing.
    runtime.evaluate(&neutral(), &session_at(1000.0, Mode::QuadPilot, true, true));
    // Release lands at t=1010: the request is emitted and the clock recorded.
    release_motion(&mut runtime, 1);
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

#[test]
fn a_deflected_scope_transfer_waits_after_the_motion_release_lands() {
    let mut runtime = with_default();
    assert!(runtime.reactivate());

    // A motion member transfer releases only that scope. Gimbal authority is
    // independent and keeps its neutral liveness stream.
    let first = runtime.evaluate(
        &deflected(),
        &session_at(1000.0, Mode::QuadPilot, true, true),
    );
    assert_eq!(first.motion_lease, Some(LeaseAction::Release));
    assert_eq!(first.lease, None);
    assert!(first.motion.is_some() && first.gimbal.is_some());

    // Next tick, acknowledgements still in flight (grants unchanged): the
    // motion release is NOT re-sent at tick rate.
    let inflight = runtime.evaluate(
        &deflected(),
        &session_at(1033.0, Mode::QuadPilot, true, true),
    );
    assert_eq!(inflight.motion_lease, None, "no duplicate release per tick");

    // Once the motion release is reflected, motion waits silently while the
    // independent gimbal scope continues its neutral stream.
    release_motion(&mut runtime, 1);
    for now_ms in [1066.0, 1100.0, 2000.0, 30_000.0] {
        let waiting = runtime.evaluate(
            &deflected(),
            &session_at(now_ms, Mode::QuadPilot, true, false),
        );
        assert_eq!(waiting.motion_lease, None, "no release without a grant");
        assert_eq!(waiting.lease, None, "gimbal authority is retained");
        assert!(waiting.motion.is_none(), "no frame rides a released lease");
        assert!(waiting.gimbal.is_some(), "gimbal liveness remains neutral");
    }
    assert_eq!(
        runtime.activation_revision(),
        1,
        "still pending while deflected"
    );
}

#[test]
fn a_lost_handover_release_is_retried_on_the_window_not_per_tick() {
    let mut runtime = with_default();
    assert!(runtime.reactivate());
    let first = runtime.evaluate(
        &deflected(),
        &session_at(1000.0, Mode::QuadPilot, true, true),
    );
    assert_eq!(first.motion_lease, Some(LeaseAction::Release));

    // The session still shows the grant well past the retry window — the
    // release write was lost. Quiet inside the window, one re-emit after it.
    let quiet = runtime.evaluate(
        &deflected(),
        &session_at(1100.0, Mode::QuadPilot, true, true),
    );
    assert_eq!(quiet.motion_lease, None, "quiet inside the retry window");
    let retry = runtime.evaluate(
        &deflected(),
        &session_at(1300.0, Mode::QuadPilot, true, true),
    );
    assert_eq!(
        retry.motion_lease,
        Some(LeaseAction::Release),
        "a lost release is retried once the window elapses"
    );
}

#[test]
fn a_gated_arm_press_is_reported_suppressed_not_silent() {
    let mut runtime = with_default();
    runtime.begin_session();
    // Prime the generation's edge baselines with nothing held, ungranted.
    runtime.evaluate(&neutral(), &session_motion(Mode::QuadPilot, false, false));
    // A fresh arm press while gated: no arm action, but a suppression report —
    // a swallowed safety press must never be indistinguishable from a dead key.
    let press = runtime.evaluate(
        &sample(&[0.0, 0.0, 0.0, 0.0], &[9]),
        &session_motion(Mode::QuadPilot, false, false),
    );
    assert!(press.motion.is_none(), "motion stays gated");
    assert!(!press.arm, "no arm action rides absent authority");
    assert!(press.arm_suppressed, "the swallowed press is reported");
    // Held: the baseline advanced, so neither an action nor a report re-fires.
    let held = runtime.evaluate(
        &sample(&[0.0, 0.0, 0.0, 0.0], &[9]),
        &session_motion(Mode::QuadPilot, false, false),
    );
    assert!(
        !held.arm && !held.arm_suppressed,
        "a held press reports once"
    );
    // Release, regrant, press again: the arm fires live with no report.
    grant(&mut runtime, AuthorityScope::Motion, 1);
    runtime.begin_control_run();
    runtime.evaluate(&neutral(), &session_motion(Mode::QuadPilot, false, true));
    let live = runtime.evaluate(
        &sample(&[0.0, 0.0, 0.0, 0.0], &[9]),
        &session_motion(Mode::QuadPilot, false, true),
    );
    assert!(
        live.arm && !live.arm_suppressed,
        "a granted press arms live"
    );
    // Disarm mirrors the same contract.
    runtime.authority_event(
        AuthorityScope::Motion,
        AuthorityEvent::Revoked { generation: 1 },
    );
    runtime.evaluate(&neutral(), &session_motion(Mode::QuadPilot, false, false));
    let disarm = runtime.evaluate(
        &sample(&[0.0, 0.0, 0.0, 0.0], &[8]),
        &session_motion(Mode::QuadPilot, false, false),
    );
    assert!(!disarm.disarm && disarm.disarm_suppressed);
}
