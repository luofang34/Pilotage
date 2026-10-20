use super::{grant, sample, session, with_default};
use crate::{
    AXIS_PITCH, AXIS_ROLL, AXIS_THROTTLE, AXIS_YAW, AuthorityEvent, AuthorityScope,
    CameraPanRouting, ControlRuntime, DirectDemand, Frame, Mode,
};

fn axis(frame: Option<&Frame>, id: u16) -> f32 {
    frame
        .unwrap()
        .axes()
        .iter()
        .find(|(key, _)| *key == id)
        .unwrap()
        .1
}

fn ready(routing: CameraPanRouting) -> ControlRuntime {
    let mut runtime = with_default();
    runtime.set_camera_pan_routing(routing);
    for _ in 0..3 {
        runtime.evaluate(&sample(&[0.0; 4], &[]), &session(Mode::QuadPilot, true));
    }
    runtime
}

#[test]
fn pitch_only_camera_pan_replaces_yaw_without_roll_or_pitch_translation() {
    for mode in [Mode::QuadPilot, Mode::QuadCruise, Mode::Fpv, Mode::Acro] {
        let mut runtime = ready(CameraPanRouting::AirframeYaw);
        for right in [-1.0, 0.0, 1.0] {
            let plan = runtime.evaluate(
                &sample(&[-1.0, 0.0, right, 1.0], &[6]),
                &session(mode, true),
            );
            assert!(plan.capture_active);
            assert_eq!(axis(plan.gimbal.as_ref(), AXIS_PITCH), -1.0);
            assert_eq!(axis(plan.gimbal.as_ref(), AXIS_YAW), 0.0);
            assert_eq!(axis(plan.motion.as_ref(), AXIS_YAW), right);
            assert_eq!(axis(plan.motion.as_ref(), AXIS_PITCH), 0.0);
            assert_eq!(axis(plan.motion.as_ref(), AXIS_THROTTLE), 0.0);
            let roll = if mode == Mode::QuadCruise { -1.0 } else { 0.0 };
            assert_eq!(axis(plan.motion.as_ref(), AXIS_ROLL), roll);
        }
        let ordinary = runtime.evaluate(&sample(&[-1.0, 0.0, 1.0, 1.0], &[]), &session(mode, true));
        let yaw = if mode == Mode::QuadCruise { 1.0 } else { -1.0 };
        assert_eq!(axis(ordinary.motion.as_ref(), AXIS_YAW), yaw);
        assert_eq!(axis(ordinary.gimbal.as_ref(), AXIS_YAW), 0.0);
    }
}

#[test]
fn camera_yaw_never_escapes_motion_authority_or_input_loss() {
    let mut runtime = ready(CameraPanRouting::AirframeYaw);
    runtime.authority_event(
        AuthorityScope::Motion,
        AuthorityEvent::Revoked { generation: 1 },
    );
    let input = sample(&[0.0, 0.0, 1.0, 1.0], &[6]);
    let plan = runtime.evaluate(&input, &session(Mode::QuadPilot, true));
    assert!(plan.motion.is_none());
    assert_eq!(axis(plan.gimbal.as_ref(), AXIS_PITCH), -1.0);
    assert_eq!(axis(plan.gimbal.as_ref(), AXIS_YAW), 0.0);
    grant(&mut runtime, AuthorityScope::Motion, 2);
    runtime.begin_control_run();
    let recovery = runtime.evaluate(&input, &session(Mode::QuadPilot, true));
    assert!(
        recovery
            .motion
            .is_none_or(|frame| frame.axes().iter().all(|(_, value)| *value == 0.0))
    );
    let mut lost = session(Mode::QuadPilot, true);
    lost.input_lost = true;
    let plan = runtime.evaluate(&input, &lost);
    assert!(plan.motion.is_none() && plan.gimbal.is_none());
}

#[test]
fn routing_change_requires_neutral_even_when_both_leases_stay_held() {
    let mut runtime = with_default();
    let input = sample(&[0.0, 0.0, 1.0, 0.0], &[6]);
    let before = runtime.evaluate(&input, &session(Mode::QuadPilot, true));
    assert_eq!(axis(before.gimbal.as_ref(), AXIS_YAW), 1.0);
    let revision = runtime.activation_revision();
    assert!(runtime.set_camera_pan_routing(CameraPanRouting::AirframeYaw));
    assert!(!runtime.set_camera_pan_routing(CameraPanRouting::AirframeYaw));
    for _ in 0..4 {
        let plan = runtime.evaluate(&input, &session(Mode::QuadPilot, true));
        assert_eq!(axis(plan.motion.as_ref(), AXIS_YAW), 0.0);
        assert_eq!(axis(plan.gimbal.as_ref(), AXIS_YAW), 0.0);
        assert_eq!(plan.lease, None);
        assert_eq!(plan.motion_lease, None);
        assert_eq!(runtime.activation_revision(), revision);
    }
    for _ in 0..3 {
        runtime.evaluate(&sample(&[0.0; 4], &[]), &session(Mode::QuadPilot, true));
    }
    assert_eq!(runtime.activation_revision(), revision.wrapping_add(1));
    let after = runtime.evaluate(&input, &session(Mode::QuadPilot, true));
    assert_eq!(axis(after.motion.as_ref(), AXIS_YAW), 1.0);
    assert_eq!(axis(after.gimbal.as_ref(), AXIS_YAW), 0.0);
}

#[test]
fn unavailable_pan_is_not_gimbal_yaw_and_r3_does_not_turn_the_airframe() {
    for route in [
        CameraPanRouting::AirframeYawUnavailable,
        CameraPanRouting::Unavailable,
    ] {
        let mut runtime = ready(route);
        let plan = runtime.evaluate(
            &sample(&[0.0, 0.0, 1.0, 0.0], &[6]),
            &session(Mode::QuadPilot, true),
        );
        assert_eq!(axis(plan.motion.as_ref(), AXIS_YAW), 0.0);
        assert_eq!(axis(plan.gimbal.as_ref(), AXIS_YAW), 0.0);
    }
    let mut runtime = ready(CameraPanRouting::AirframeYaw);
    let plan = runtime.evaluate(&sample(&[0.0; 4], &[11]), &session(Mode::QuadPilot, true));
    assert!(!plan.gimbal.unwrap().edges().is_empty());
    assert_eq!(axis(plan.motion.as_ref(), AXIS_YAW), 0.0);
}

#[test]
fn direct_demands_are_not_reinterpreted_and_a_new_session_forgets_routing() {
    let mut runtime = ready(CameraPanRouting::AirframeYaw);
    let mut input = sample(&[0.0, 0.0, -1.0, 0.0], &[6]);
    input.direct = Some(DirectDemand {
        yaw: 0.4,
        ..Default::default()
    });
    let plan = runtime.evaluate(&input, &session(Mode::QuadPilot, true));
    assert_eq!(axis(plan.motion.as_ref(), AXIS_YAW), 0.4);
    runtime.begin_session();
    assert_eq!(runtime.camera_pan_routing(), CameraPanRouting::GimbalYaw);
}

#[test]
fn a_camera_route_update_preserves_the_pending_scheme() {
    let mut runtime = with_default();
    let candidate = crate::ProfileRuntime::compile(super::SECOND_PROFILE.as_bytes()).unwrap();
    let digest = candidate.digest();
    runtime.activate(candidate);
    runtime.set_camera_pan_routing(CameraPanRouting::AirframeYaw);
    for _ in 0..3 {
        runtime.evaluate(&sample(&[0.0; 4], &[]), &session(Mode::QuadPilot, true));
    }
    assert_eq!(runtime.active_profile_id(), "test.gimbal.alt");
    assert_eq!(runtime.active_profile_digest(), digest);
    let plan = runtime.evaluate(
        &sample(&[1.0, 1.0, 0.0, 0.0], &[4]),
        &session(Mode::QuadPilot, true),
    );
    assert_eq!(axis(plan.motion.as_ref(), AXIS_YAW), 1.0);
    assert_eq!(axis(plan.gimbal.as_ref(), AXIS_PITCH), -1.0);
}

#[test]
fn gimbal_grant_or_loss_never_reinterprets_a_held_camera_stick() {
    let mut runtime = ready(CameraPanRouting::AirframeYaw);
    let input = sample(&[0.0, 0.0, 1.0, 0.0], &[6]);
    runtime.evaluate(&input, &session(Mode::QuadPilot, true));
    runtime.authority_event(
        AuthorityScope::Gimbal,
        AuthorityEvent::Revoked { generation: 1 },
    );
    for _ in 0..3 {
        let plan = runtime.evaluate(&input, &session(Mode::QuadPilot, true));
        assert_eq!(axis(plan.motion.as_ref(), AXIS_YAW), 0.0);
        assert_eq!(axis(plan.motion.as_ref(), AXIS_ROLL), 0.0);
        assert!(plan.gimbal.is_none());
    }
    for _ in 0..3 {
        runtime.evaluate(&sample(&[0.0; 4], &[]), &session(Mode::QuadPilot, true));
    }
    let flight = runtime.evaluate(&input, &session(Mode::QuadPilot, true));
    assert_eq!(axis(flight.motion.as_ref(), AXIS_ROLL), 1.0);
    grant(&mut runtime, AuthorityScope::Gimbal, 2);
    let held = runtime.evaluate(&input, &session(Mode::QuadPilot, true));
    assert_eq!(axis(held.motion.as_ref(), AXIS_YAW), 0.0);
    assert_eq!(axis(held.motion.as_ref(), AXIS_ROLL), 0.0);
    for _ in 0..3 {
        runtime.evaluate(&sample(&[0.0; 4], &[]), &session(Mode::QuadPilot, true));
    }
    let camera = runtime.evaluate(&input, &session(Mode::QuadPilot, true));
    assert_eq!(axis(camera.motion.as_ref(), AXIS_YAW), 1.0);
    assert_eq!(axis(camera.motion.as_ref(), AXIS_ROLL), 0.0);
    let revision = runtime.activation_revision();
    runtime.authority_event(
        AuthorityScope::Gimbal,
        AuthorityEvent::Revoked { generation: 1 },
    );
    let late = runtime.evaluate(&input, &session(Mode::QuadPilot, true));
    assert_eq!(axis(late.motion.as_ref(), AXIS_YAW), 1.0);
    assert_eq!(runtime.activation_revision(), revision);
}
