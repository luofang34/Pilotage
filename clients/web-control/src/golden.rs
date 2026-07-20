#![allow(clippy::expect_used, clippy::panic)]

//! Golden vectors shared with the wasm suite
//! (clients/web/control-runtime.test.mjs). The two assert the SAME plan for
//! the SAME raw sample and the built-in default profile, so a native/wasm
//! divergence reddens one of them. Keep the vectors here and there in lock
//! step.

use crate::DEFAULT_PROFILE_BYTES;
use crate::plan::{AXIS_PITCH, AXIS_THROTTLE, AXIS_YAW, LeaseAction};
use crate::profile::ProfileRuntime;
use crate::runtime::ControlRuntime;
use crate::sample::{ButtonSample, Mode, RawSample, SessionState};

fn runtime() -> ControlRuntime {
    let mut runtime = ControlRuntime::new();
    let profile = ProfileRuntime::compile(DEFAULT_PROFILE_BYTES).expect("default compiles");
    let plan = runtime.activate(profile);
    assert_eq!(
        plan.activation_revision, 1,
        "the default activates at revision 1"
    );
    runtime
}

fn pad(axes: &[f32], pressed: &[usize]) -> RawSample {
    RawSample {
        axes: axes.to_vec(),
        buttons: (0..16)
            .map(|i| ButtonSample {
                pressed: pressed.contains(&i),
                value: if pressed.contains(&i) { 1.0 } else { 0.0 },
            })
            .collect(),
    }
}

fn session(mode: Mode, granted: bool, denied: bool) -> SessionState {
    SessionState {
        generation: 1,
        now_ms: 100_000.0,
        mode,
        connected: true,
        lease_granted: granted,
        lease_denied: denied,
        motion_granted: true,
    }
}

fn axis(frame: &Option<crate::plan::Frame>, id: u16) -> f32 {
    frame
        .as_ref()
        .and_then(|f| f.axes().iter().find(|(a, _)| *a == id))
        .map_or(f32::NAN, |(_, v)| *v)
}

#[test]
fn golden_vectors() {
    let mut rt = runtime();

    // Vector 1: LT + right stick full → gimbal rates (pitch inverted); flight
    // sees the captured right stick as neutral.
    let v1 = rt.evaluate(
        &pad(&[0.0, 0.0, 1.0, 1.0], &[6]),
        &session(Mode::QuadPilot, true, false),
    );
    assert_eq!(axis(&v1.gimbal, AXIS_PITCH), -1.0, "v1 gimbal pitch");
    assert_eq!(axis(&v1.gimbal, AXIS_YAW), 1.0, "v1 gimbal yaw");
    assert_eq!(
        axis(&v1.motion, crate::plan::AXIS_ROLL),
        0.0,
        "v1 flight roll masked"
    );
    assert_eq!(axis(&v1.motion, AXIS_PITCH), 0.0, "v1 flight pitch masked");
    assert_eq!(v1.lease, None, "v1 no lease while granted");

    // Vector 2: a fresh R3 recenters exactly once.
    let r3 = pad(&[0.0, 0.0, 0.0, 0.0], &[11]);
    let first = rt.evaluate(&r3, &session(Mode::QuadPilot, true, false));
    assert!(
        first.gimbal.as_ref().is_some_and(|f| !f.edges().is_empty()),
        "v2 a fresh R3 recenters"
    );
    let second = rt.evaluate(&r3, &session(Mode::QuadPilot, true, false));
    assert!(
        second.gimbal.as_ref().is_some_and(|f| f.edges().is_empty()),
        "v2 holding R3 does not re-recenter"
    );

    // Vector 3: a flight mode with no lease requests it; no gimbal frame.
    let mut fresh = runtime();
    let v3 = fresh.evaluate(
        &pad(&[0.0, 0.0, 0.0, 0.0], &[]),
        &session(Mode::QuadPilot, false, false),
    );
    assert_eq!(
        v3.lease,
        Some(LeaseAction::Request),
        "v3 requests the lease"
    );
    assert!(v3.gimbal.is_none(), "v3 no gimbal frame without a lease");

    // Vector 4: rover releases a held lease.
    let v4 = fresh.evaluate(
        &pad(&[0.0, 0.0, 0.0, 0.0], &[]),
        &session(Mode::Rover, true, false),
    );
    assert_eq!(v4.lease, Some(LeaseAction::Release), "v4 rover releases");

    // Vector 5: keyboard W is axis1 = -1 (left_y); throttle = -left_y = climb.
    let v5 = fresh.evaluate(
        &pad(&[0.0, -1.0, 0.0, 0.0], &[]),
        &session(Mode::QuadPilot, true, false),
    );
    assert_eq!(axis(&v5.motion, AXIS_THROTTLE), 1.0, "v5 W commands climb");
}
