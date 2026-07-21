#![allow(clippy::expect_used, clippy::panic)]

use super::ControlRuntime;
use crate::DEFAULT_PROFILE_BYTES;
use crate::plan::{AXIS_PITCH, AXIS_YAW, GIMBAL_NEUTRAL_BUTTON, LeaseAction};
use crate::profile::ProfileRuntime;
use crate::sample::{ButtonSample, Mode, RawSample, SessionState};

// The motion-lease reacquisition tests share these helpers but would push this
// file past the module size limit, so they live in a sibling submodule.
mod motion;

/// A second profile with DIFFERENT bindings: the modifier and reset move to
/// buttons 4/5 and the gimbal reads the LEFT stick (axes 0/1). Activating it
/// through the same public API is what proves future restoration is a
/// source change, not an architectural rewrite.
const SECOND_PROFILE: &str = r#"{
  "schema_version": 1,
  "revision": 9,
  "id": "test.gimbal.alt",
  "gimbal": {
    "modifier_button": 4,
    "reset_button": 5,
    "pitch": { "source_index": 1, "logical": "pitch", "invert": true, "deadzone": 0.1, "expo": 0.0, "calibration": { "min": -1.0, "center": 0.0, "max": 1.0 } },
    "yaw":   { "source_index": 0, "logical": "yaw",   "invert": false, "deadzone": 0.1, "expo": 0.0, "calibration": { "min": -1.0, "center": 0.0, "max": 1.0 } }
  },
  "flight": {
    "arm_button": 9, "disarm_button": 8,
    "left_x": 0, "left_y": 1, "right_x": 2, "right_y": 3,
    "trigger_left": 6, "trigger_right": 7, "deadzone": 0.06, "expo": 0.0
  }
}"#;

fn sample(axes: &[f32], pressed: &[usize]) -> RawSample {
    let max_button = pressed.iter().copied().max().unwrap_or(0) + 2;
    let buttons = (0..max_button)
        .map(|i| ButtonSample {
            pressed: pressed.contains(&i),
            value: if pressed.contains(&i) { 1.0 } else { 0.0 },
        })
        .collect();
    RawSample {
        axes: axes.to_vec(),
        buttons,
    }
}

fn session(mode: Mode, granted: bool) -> SessionState {
    session_gen(1, mode, granted)
}

fn session_gen(generation: u32, mode: Mode, granted: bool) -> SessionState {
    SessionState {
        generation,
        now_ms: 100_000.0,
        mode,
        connected: true,
        lease_granted: granted,
        lease_denied: false,
        motion_granted: true,
        motion_denied: false,
        motion_recovered: true,
    }
}

fn with_default() -> ControlRuntime {
    let mut runtime = ControlRuntime::new();
    let profile = ProfileRuntime::compile(DEFAULT_PROFILE_BYTES).expect("compiles");
    let plan = runtime.activate(profile);
    assert!(plan.installed, "first activation installs immediately");
    assert_eq!(plan.activation_revision, 1);
    assert!(runtime.is_active());
    runtime
}

fn gimbal_rate(plan: &crate::plan::ControlPlan, axis: u16) -> f32 {
    let frame = plan.gimbal.as_ref().expect("gimbal frame present");
    frame
        .axes()
        .iter()
        .find(|(id, _)| *id == axis)
        .map(|(_, v)| *v)
        .expect("axis present")
}

#[test]
fn lt_engages_the_quasimode_and_masks_flight() {
    let mut runtime = with_default();
    // LT (button 6) held, right stick full (axes 2=+1 yaw, 3=+1 pitch-down).
    let plan = runtime.evaluate(
        &sample(&[0.0, 0.0, 1.0, 1.0], &[6]),
        &session(Mode::QuadPilot, true),
    );
    assert_eq!(gimbal_rate(&plan, AXIS_PITCH), -1.0, "camera-down demand");
    assert_eq!(gimbal_rate(&plan, AXIS_YAW), 1.0, "camera-right demand");
    // Flight sees the captured right stick as neutral (masked): roll (right x)
    // and flight pitch (right y) are zero despite the stick being full.
    let motion = plan.motion.as_ref().expect("motion frame");
    for (id, value) in motion.axes() {
        if *id == crate::plan::AXIS_ROLL || *id == AXIS_PITCH {
            assert_eq!(*value, 0.0, "captured axis {id} is neutral to flight");
        }
    }
}

#[test]
fn r3_produces_exactly_one_recenter_edge() {
    let mut runtime = with_default();
    // Prime the session generation with a neutral tick (no button held), so the
    // baseline seeding does not swallow the genuine press under test.
    runtime.evaluate(
        &sample(&[0.0, 0.0, 0.0, 0.0], &[]),
        &session(Mode::QuadPilot, true),
    );
    let held_r3 = sample(&[0.0, 0.0, 0.0, 0.0], &[11]);
    let first = runtime.evaluate(&held_r3, &session(Mode::QuadPilot, true));
    let edges = first.gimbal.as_ref().expect("gimbal").edges();
    assert!(
        edges.contains(&(GIMBAL_NEUTRAL_BUTTON, crate::plan::BUTTON_EDGE_PRESSED)),
        "a fresh R3 press recenters"
    );
    let second = runtime.evaluate(&held_r3, &session(Mode::QuadPilot, true));
    assert!(
        second.gimbal.as_ref().expect("gimbal").edges().is_empty(),
        "holding R3 does not re-fire"
    );
}

#[test]
fn a_grant_does_not_synthesize_an_r3_edge() {
    let mut runtime = with_default();
    let held_r3 = sample(&[0.0, 0.0, 0.0, 0.0], &[11]);
    // R3 held while the lease is not yet granted: baseline advances, no frame.
    let ungranted = runtime.evaluate(&held_r3, &session(Mode::QuadPilot, false));
    assert!(
        ungranted.gimbal.is_none(),
        "no gimbal frame without a lease"
    );
    // The lease is granted with R3 still held: no fresh recenter edge.
    let granted = runtime.evaluate(&held_r3, &session(Mode::QuadPilot, true));
    assert!(
        granted.gimbal.as_ref().expect("gimbal").edges().is_empty(),
        "a lease grant with R3 held is not a fresh edge"
    );
}

#[test]
fn flight_requests_the_lease_and_rover_releases_it() {
    let mut runtime = with_default();
    let neutral = sample(&[0.0, 0.0, 0.0, 0.0], &[]);
    let request = runtime.evaluate(&neutral, &session(Mode::QuadPilot, false));
    assert_eq!(request.lease, Some(LeaseAction::Request));
    let rover = runtime.evaluate(&neutral, &session(Mode::Rover, true));
    assert_eq!(rover.lease, Some(LeaseAction::Release));
}

#[test]
fn a_second_profile_takes_effect_only_after_a_neutral_transaction() {
    let mut runtime = with_default();
    let held = sample(&[0.0, 0.0, 1.0, 1.0], &[6]); // LT + right stick, default capture
    // Default active: the right stick streams gimbal rates.
    let before = runtime.evaluate(&held, &session(Mode::QuadPilot, true));
    assert_eq!(gimbal_rate(&before, AXIS_YAW), 1.0);

    // Activate the alternate profile while the captured controls are held.
    let candidate = ProfileRuntime::compile(SECOND_PROFILE.as_bytes()).expect("compiles");
    let activation = runtime.activate(candidate);
    assert!(
        !activation.installed,
        "handover deferred while controls held"
    );
    assert!(activation.release_gimbal_lease);

    // While controls stay held the runtime emits ONLY the neutral handover —
    // neither the old rates nor the new binding takes effect.
    let during = runtime.evaluate(&held, &session(Mode::QuadPilot, true));
    assert_eq!(
        gimbal_rate(&during, AXIS_YAW),
        0.0,
        "output is neutral mid-handover"
    );
    assert_eq!(during.lease, Some(LeaseAction::Release));
    assert_eq!(runtime.activation_revision(), 1, "not installed yet");

    // Release everything: the captured controls are neutral, so the candidate
    // installs and the revision advances.
    let neutral = sample(&[0.0, 0.0, 0.0, 0.0], &[]);
    let _settle = runtime.evaluate(&neutral, &session(Mode::QuadPilot, true));
    assert_eq!(runtime.activation_revision(), 2, "installed after neutral");

    // Now the ALT bindings drive: LT (button 6) no longer engages; button 4
    // does, and the gimbal reads the LEFT stick (axes 0/1).
    let old_input = runtime.evaluate(
        &sample(&[0.0, 0.0, 1.0, 1.0], &[6]),
        &session(Mode::QuadPilot, true),
    );
    assert_eq!(
        gimbal_rate(&old_input, AXIS_YAW),
        0.0,
        "old modifier no longer engages"
    );
    let new_input = runtime.evaluate(
        &sample(&[1.0, 0.0, 0.0, 0.0], &[4]),
        &session(Mode::QuadPilot, true),
    );
    assert_eq!(
        gimbal_rate(&new_input, AXIS_YAW),
        1.0,
        "alt binding reads the left stick"
    );
}

#[test]
fn a_button_held_across_activation_does_not_fire_a_fresh_edge() {
    let mut runtime = with_default();
    // Establish R3 baseline with the lease granted (a genuine edge fires once).
    let held_r3 = sample(&[0.0, 0.0, 0.0, 0.0], &[11]);
    let _seed = runtime.evaluate(&held_r3, &session(Mode::QuadPilot, true));

    // Activate a new profile while R3 stays held, then release to install.
    let candidate = ProfileRuntime::compile(DEFAULT_PROFILE_BYTES).expect("compiles");
    runtime.activate(candidate);
    let neutral = sample(&[0.0, 0.0, 0.0, 0.0], &[11]); // R3 still held, sticks centered
    let _install = runtime.evaluate(&neutral, &session(Mode::QuadPilot, true));

    // With R3 held straight through the install, no recenter edge is synthesized.
    let after = runtime.evaluate(&held_r3, &session(Mode::QuadPilot, true));
    assert!(
        after.gimbal.as_ref().expect("gimbal").edges().is_empty(),
        "R3 held across activation is not a fresh edge"
    );
}

#[test]
fn a_control_held_across_reconnect_fires_no_edge() {
    let mut runtime = with_default();
    // Generation 1: arm (button 9) and R3 (button 11) held; the first tick of
    // a generation primes the baselines and fires nothing.
    let held = sample(&[0.0, 0.0, 0.0, 0.0], &[9, 11]);
    runtime.evaluate(&held, &session_gen(1, Mode::QuadPilot, true));

    // Disconnect (no ticks evaluated while disconnected), then reconnect as a
    // NEW generation with both controls STILL held. The first tick under the
    // new generation must fire no arm and no recenter.
    let reconnect = runtime.evaluate(&held, &session_gen(2, Mode::QuadPilot, true));
    assert!(!reconnect.arm, "arm held across reconnect fires no arm");
    assert!(
        reconnect
            .gimbal
            .as_ref()
            .expect("gimbal")
            .edges()
            .is_empty(),
        "R3 held across reconnect fires no recenter"
    );
    // Still held on the next tick: still nothing.
    let holding = runtime.evaluate(&held, &session_gen(2, Mode::QuadPilot, true));
    assert!(!holding.arm && holding.gimbal.as_ref().expect("gimbal").edges().is_empty());

    // Release, then press again: exactly one arm and one recenter.
    runtime.evaluate(
        &sample(&[0.0, 0.0, 0.0, 0.0], &[]),
        &session_gen(2, Mode::QuadPilot, true),
    );
    let press = runtime.evaluate(&held, &session_gen(2, Mode::QuadPilot, true));
    assert!(press.arm, "a fresh press after release arms exactly once");
    assert!(
        press
            .gimbal
            .as_ref()
            .expect("gimbal")
            .edges()
            .contains(&(GIMBAL_NEUTRAL_BUTTON, crate::plan::BUTTON_EDGE_PRESSED)),
        "a fresh press after release recenters exactly once"
    );
}

#[test]
fn arm_and_disarm_are_typed_and_follow_the_profile_binding() {
    // A profile that rebinds arm/disarm to buttons 4/5 must still fire the
    // TYPED arm/disarm actions — the runtime never emits a physical index.
    const REBOUND: &str = r#"{
      "schema_version": 1, "revision": 1, "id": "test.rebind",
      "gimbal": {
        "modifier_button": 6, "reset_button": 11,
        "pitch": { "source_index": 3, "logical": "pitch", "invert": true, "deadzone": 0.1, "expo": 0.0, "calibration": { "min": -1.0, "center": 0.0, "max": 1.0 } },
        "yaw":   { "source_index": 2, "logical": "yaw",   "invert": false, "deadzone": 0.1, "expo": 0.0, "calibration": { "min": -1.0, "center": 0.0, "max": 1.0 } }
      },
      "flight": { "arm_button": 4, "disarm_button": 5, "left_x": 0, "left_y": 1, "right_x": 2, "right_y": 3, "trigger_left": 6, "trigger_right": 7, "deadzone": 0.06, "expo": 0.0 }
    }"#;
    let mut runtime = ControlRuntime::new();
    runtime.activate(ProfileRuntime::compile(REBOUND.as_bytes()).expect("compiles"));
    runtime.evaluate(
        &sample(&[0.0, 0.0, 0.0, 0.0], &[]),
        &session(Mode::QuadPilot, true),
    );
    let armed = runtime.evaluate(
        &sample(&[0.0, 0.0, 0.0, 0.0], &[4]),
        &session(Mode::QuadPilot, true),
    );
    assert!(
        armed.arm,
        "the rebound arm button (4) fires the typed arm action"
    );
    assert!(!armed.disarm);
    let disarmed = runtime.evaluate(
        &sample(&[0.0, 0.0, 0.0, 0.0], &[5]),
        &session(Mode::QuadPilot, true),
    );
    assert!(
        disarmed.disarm,
        "the rebound disarm button (5) fires the typed disarm action"
    );
}

#[test]
fn a_profile_swap_waits_for_the_candidate_controls_to_be_neutral_too() {
    // The alternate profile's modifier is button 4 (the default's is 6). While
    // button 4 is held, a swap must NOT install: the candidate binds button 4,
    // so installing would change its meaning under a held control — even though
    // button 4 is neutral for the currently-active profile.
    let mut runtime = with_default();
    runtime.evaluate(
        &sample(&[0.0, 0.0, 0.0, 0.0], &[]),
        &session(Mode::QuadPilot, true),
    );
    let candidate = ProfileRuntime::compile(SECOND_PROFILE.as_bytes()).expect("compiles");
    runtime.activate(candidate);

    let held_alt_modifier = sample(&[0.0, 0.0, 0.0, 0.0], &[4]);
    let during = runtime.evaluate(&held_alt_modifier, &session(Mode::QuadPilot, true));
    assert_eq!(
        runtime.activation_revision(),
        1,
        "the candidate's own control held blocks install"
    );
    assert_eq!(
        during.motion_lease,
        Some(LeaseAction::Release),
        "the motion lease is cycled too"
    );

    // Release: the union of both profiles' controls is neutral, so it installs.
    // The full motion-lease reacquisition handshake has its own test below;
    // here it is enough that the install lands.
    runtime.evaluate(
        &sample(&[0.0, 0.0, 0.0, 0.0], &[]),
        &session(Mode::QuadPilot, true),
    );
    assert_eq!(
        runtime.activation_revision(),
        2,
        "installs once the union is neutral"
    );
}

#[test]
fn lt_does_not_suppress_flight_without_a_gimbal_lease() {
    let mut runtime = with_default();
    runtime.evaluate(
        &sample(&[0.0, 0.0, 0.0, 0.0], &[]),
        &session(Mode::QuadPilot, false),
    );
    // LT (button 6) held with the lease DENIED, left stick forward + right
    // stick deflected. LT must NOT capture, so flight keeps flying on both
    // sticks and there is no gimbal frame or capture — never a silent loss.
    let plan = runtime.evaluate(
        &sample(&[0.0, -1.0, 1.0, 1.0], &[6]),
        &session(Mode::QuadPilot, false),
    );
    let motion = plan.motion.as_ref().expect("motion");
    let axis = |id: u16| {
        motion
            .axes()
            .iter()
            .find(|(a, _)| *a == id)
            .map(|(_, v)| *v)
            .expect("axis")
    };
    assert_eq!(
        axis(crate::plan::AXIS_THROTTLE),
        1.0,
        "left stick still climbs with no lease"
    );
    assert_eq!(
        axis(crate::plan::AXIS_ROLL),
        1.0,
        "right stick still rolls (LT did not capture)"
    );
    assert!(plan.gimbal.is_none(), "no gimbal frame without a lease");
    assert!(!plan.capture_active, "no capture without a lease");
}

#[test]
fn capture_active_is_reported_even_at_centered_stick() {
    let mut runtime = with_default();
    runtime.evaluate(
        &sample(&[0.0, 0.0, 0.0, 0.0], &[]),
        &session(Mode::QuadPilot, true),
    );
    // LT (button 6) held but the right stick centered: rates are zero, yet the
    // quasimode IS capturing — the HUD must be able to show it.
    let plan = runtime.evaluate(
        &sample(&[0.0, 0.0, 0.0, 0.0], &[6]),
        &session(Mode::QuadPilot, true),
    );
    assert!(
        plan.capture_active,
        "LT held reports capture even at centered stick"
    );
    let released = runtime.evaluate(
        &sample(&[0.0, 0.0, 0.0, 0.0], &[]),
        &session(Mode::QuadPilot, true),
    );
    assert!(!released.capture_active, "no capture once LT is released");
}

#[test]
fn a_device_reselect_reseeds_edge_baselines() {
    let mut runtime = with_default();
    // Settle a released arm baseline under the running generation.
    runtime.evaluate(&sample(&[0.0; 4], &[]), &session(Mode::QuadPilot, true));
    // The device mapping changed (pad swap): the next tick seeds baselines
    // from the held state, so an already-pressed arm fires nothing.
    runtime.reseed_edge_baselines();
    let held = runtime.evaluate(&sample(&[0.0; 4], &[9]), &session(Mode::QuadPilot, true));
    assert!(!held.arm, "arm held through a device swap fires no edge");
    // A genuine release-then-press on the same mapping still fires once.
    runtime.evaluate(&sample(&[0.0; 4], &[]), &session(Mode::QuadPilot, true));
    let pressed = runtime.evaluate(&sample(&[0.0; 4], &[9]), &session(Mode::QuadPilot, true));
    assert!(pressed.arm, "a fresh arm after release fires once");
}
