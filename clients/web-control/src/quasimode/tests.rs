#![allow(clippy::expect_used, clippy::panic)]

use super::{
    GimbalDemand, LEASE_DEBOUNCE_MS, LeasePlan, frame_plan, gimbal_demand, lease_plan,
    modifier_held, reset_edge,
};
use crate::DEFAULT_PROFILE_BYTES;
use crate::profile::{CompiledProfile, ProfileRuntime};
use crate::sample::{ButtonSample, Mode, RawSample, SessionState};

fn profile() -> CompiledProfile {
    ProfileRuntime::compile(DEFAULT_PROFILE_BYTES).expect("default compiles")
}

/// A sample with the given axes and a set of pressed button indices.
fn sample(axes: &[f32], pressed: &[usize]) -> RawSample {
    let max_button = pressed.iter().copied().max().unwrap_or(0) + 1;
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

fn session(mode: Mode, granted: bool, denied: bool, now_ms: f64) -> SessionState {
    SessionState {
        generation: 1,
        now_ms,
        mode,
        connected: true,
        lease_granted: granted,
        lease_denied: denied,
    }
}

#[test]
fn the_modifier_engages_on_press_or_half_travel() {
    let p = profile();
    assert!(!modifier_held(&sample(&[], &[]), &p.gimbal));
    assert!(modifier_held(&sample(&[], &[6]), &p.gimbal));
    let mut analog = sample(&[], &[]);
    analog.buttons = vec![ButtonSample::default(); 7];
    analog.buttons[6] = ButtonSample {
        pressed: false,
        value: 0.6,
    };
    assert!(
        modifier_held(&analog, &p.gimbal),
        "past half travel engages"
    );
}

#[test]
fn the_right_stick_drives_gimbal_with_pitch_inverted() {
    let p = profile();
    // Axis 3 (right stick Y) full down (+1) inverts to camera-down (-1);
    // axis 2 (right stick X) full right (+1) is yaw +1.
    let demand = gimbal_demand(&sample(&[0.0, 0.0, 1.0, 1.0], &[]), &p.gimbal);
    assert_eq!(demand.pitch, -1.0, "stick down = camera down (inverted)");
    assert_eq!(demand.yaw, 1.0, "stick right = camera right");
    // Inside the 0.1 deadzone reads exactly neutral.
    let dz = gimbal_demand(&sample(&[0.0, 0.0, 0.05, 0.05], &[]), &p.gimbal);
    assert_eq!(dz.pitch, 0.0);
    assert_eq!(dz.yaw, 0.0);
}

#[test]
fn the_r3_edge_fires_once_and_never_across_reactivation() {
    // Held while inactive advances the baseline but fires no edge.
    let a = reset_edge(true, false, false);
    assert!(!a.edge);
    assert!(a.baseline, "baseline advances even while inactive");
    // Re-activating with R3 still held is NOT a fresh edge.
    let b = reset_edge(true, a.baseline, true);
    assert!(!b.edge, "held across (re)activation does not fire");
    // A genuine press while active fires exactly once.
    let press = reset_edge(true, false, true);
    assert!(press.edge);
    let hold = reset_edge(true, press.baseline, true);
    assert!(!hold.edge, "holding does not re-fire");
}

#[test]
fn the_frame_plan_streams_holds_and_exits_neutral() {
    assert_eq!(
        frame_plan(false, false, false, GimbalDemand::default()),
        None,
        "idle with no stream yields no active frame"
    );
    let active = frame_plan(
        true,
        false,
        false,
        GimbalDemand {
            pitch: 0.5,
            yaw: -0.5,
        },
    )
    .expect("held streams");
    assert_eq!(
        (active.pitch, active.yaw, active.streaming),
        (0.5, -0.5, true)
    );
    // Exit: not held but streaming -> one trailing neutral, streaming false.
    let exit = frame_plan(
        false,
        false,
        true,
        GimbalDemand {
            pitch: 0.5,
            yaw: 0.5,
        },
    )
    .expect("trailing neutral");
    assert_eq!((exit.pitch, exit.yaw, exit.streaming), (0.0, 0.0, false));
    // A recenter edge alone produces a neutral frame carrying the recenter.
    let recenter = frame_plan(false, true, false, GimbalDemand::default()).expect("recenter");
    assert!(recenter.recenter);
}

#[test]
fn the_lease_plan_requests_debounces_and_releases() {
    let p_request = lease_plan(
        &session(Mode::QuadPilot, false, false, 5000.0),
        f64::NEG_INFINITY,
    );
    assert_eq!(p_request, LeasePlan::Request, "a flight mode requests");
    // A request one millisecond inside the debounce window is suppressed;
    // one just outside it is allowed.
    let inside = lease_plan(
        &session(Mode::QuadPilot, false, false, 5000.0),
        5000.0 - LEASE_DEBOUNCE_MS + 1.0,
    );
    assert_eq!(inside, LeasePlan::None, "a fresh request debounces");
    let outside = lease_plan(
        &session(Mode::QuadPilot, false, false, 5000.0),
        5000.0 - LEASE_DEBOUNCE_MS - 1.0,
    );
    assert_eq!(
        outside,
        LeasePlan::Request,
        "past the debounce window it re-requests"
    );
    assert_eq!(
        lease_plan(
            &session(Mode::QuadPilot, true, false, 5000.0),
            f64::NEG_INFINITY
        ),
        LeasePlan::None,
        "a granted lease is not re-requested"
    );
    assert_eq!(
        lease_plan(
            &session(Mode::QuadPilot, false, true, 5000.0),
            f64::NEG_INFINITY
        ),
        LeasePlan::None,
        "a denied scope is never re-requested"
    );
    assert_eq!(
        lease_plan(
            &session(Mode::Rover, true, false, 5000.0),
            f64::NEG_INFINITY
        ),
        LeasePlan::Release,
        "rover releases a held lease"
    );
}
