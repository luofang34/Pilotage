//! The typed-command rejection matrix and the legacy-translation contract.

#![allow(clippy::expect_used, clippy::panic)]

use pilotage_adapter_api::{
    ActionCapability, IntentCapability, LegacyAxisRoute, LegacyCommandMap, ScopeDescriptor,
};
use pilotage_protocol::{
    ActionKind, ButtonEdge, ControlAction, ControlIntent, ControlPayload, FrameRejectionReason,
    Generation, GimbalRateIntent, IntentFamily, LogicalAxisId, LogicalButtonId, ModeTarget,
    ReferenceFrame, ScopeId, ScopedControlFrame, SequenceNum, SessionId, VehicleId, VelocityIntent,
};
use pilotage_timing::MonoTimestamp;

use super::gate_frame;

fn motion_scope() -> ScopeDescriptor {
    ScopeDescriptor {
        scope: ScopeId::new("vehicle.motion"),
        axes: vec![],
        intents: vec![IntentCapability {
            family: IntentFamily::Velocity,
            frames: vec![ReferenceFrame::BodyFrd],
            max_linear: 3.0,
            max_vertical: 1.5,
            max_angular: 0.9,
        }],
        actions: vec![
            ActionCapability {
                action: ActionKind::Arm,
                mode_targets: vec![],
            },
            ActionCapability {
                action: ActionKind::Disarm,
                mode_targets: vec![],
            },
            ActionCapability {
                action: ActionKind::ModeRequest,
                mode_targets: vec![ModeTarget::CameraVelocity],
            },
        ],
        legacy: Some(LegacyCommandMap::Velocity {
            vx: Some(LegacyAxisRoute { axis: 1, sign: 1.0 }),
            vy: Some(LegacyAxisRoute { axis: 0, sign: 1.0 }),
            vz: Some(LegacyAxisRoute {
                axis: 2,
                sign: -1.0,
            }),
            yaw_rate: Some(LegacyAxisRoute { axis: 3, sign: 1.0 }),
            arm_button: Some(0),
            disarm_button: Some(1),
            reset_button: None,
        }),
    }
}

fn gimbal_scope() -> ScopeDescriptor {
    ScopeDescriptor {
        scope: ScopeId::new("vehicle.gimbal"),
        axes: vec![],
        intents: vec![IntentCapability {
            family: IntentFamily::GimbalRate,
            frames: vec![],
            max_linear: 0.0,
            max_vertical: 0.0,
            max_angular: 0.8,
        }],
        actions: vec![ActionCapability {
            action: ActionKind::GimbalRecenter,
            mode_targets: vec![],
        }],
        legacy: Some(LegacyCommandMap::GimbalRate {
            pitch: Some(LegacyAxisRoute { axis: 1, sign: 1.0 }),
            yaw: Some(LegacyAxisRoute { axis: 3, sign: 1.0 }),
            recenter_button: Some(0),
        }),
    }
}

fn base_frame() -> ScopedControlFrame {
    ScopedControlFrame {
        session: SessionId::new(1),
        vehicle: VehicleId::new(1),
        scope: ScopeId::new("vehicle.motion"),
        generation: Generation::new(1),
        sequence: SequenceNum::new(1),
        sampled_at: MonoTimestamp::from_nanos(0),
        profile_revision: 1,
        activation_revision: 1,
        payload: ControlPayload::default(),
        intent: None,
        actions: vec![],
    }
}

fn velocity(vx: f32, vy: f32, vz: f32, yaw_rate: f32) -> ControlIntent {
    ControlIntent::Velocity(VelocityIntent {
        frame: ReferenceFrame::BodyFrd,
        vx,
        vy,
        vz,
        yaw_rate,
    })
}

#[test]
fn an_empty_frame_is_rejected() {
    assert_eq!(
        gate_frame(&base_frame(), &motion_scope()),
        Err(FrameRejectionReason::EmptyCommand)
    );
}

#[test]
fn a_dual_representation_frame_is_rejected() {
    let mut frame = base_frame();
    frame.intent = Some(velocity(1.0, 0.0, 0.0, 0.0));
    frame.payload.axes = vec![(LogicalAxisId::new(1), 0.5)];
    assert_eq!(
        gate_frame(&frame, &motion_scope()),
        Err(FrameRejectionReason::DualCommand)
    );
}

#[test]
fn a_typed_velocity_within_limits_passes_unchanged() {
    let mut frame = base_frame();
    frame.intent = Some(velocity(3.0, -3.0, 1.5, -0.9));
    frame.actions = vec![ControlAction::Arm];
    let gated = gate_frame(&frame, &motion_scope()).expect("within the advertisement");
    assert_eq!(gated.intent, frame.intent);
    assert_eq!(gated.actions, frame.actions);
    assert!(!gated.carries_payload());
}

#[test]
fn an_unadvertised_family_is_rejected() {
    let mut frame = base_frame();
    frame.intent = Some(ControlIntent::GimbalRate(GimbalRateIntent {
        pitch_rate: 0.1,
        yaw_rate: 0.0,
    }));
    assert_eq!(
        gate_frame(&frame, &motion_scope()),
        Err(FrameRejectionReason::UnsupportedIntent)
    );
}

#[test]
fn an_unadvertised_reference_frame_is_rejected() {
    let mut frame = base_frame();
    frame.intent = Some(ControlIntent::Velocity(VelocityIntent {
        frame: ReferenceFrame::LocalNed,
        vx: 1.0,
        vy: 0.0,
        vz: 0.0,
        yaw_rate: 0.0,
    }));
    assert_eq!(
        gate_frame(&frame, &motion_scope()),
        Err(FrameRejectionReason::UnsupportedIntent)
    );
}

#[test]
fn components_beyond_the_advertised_limits_are_rejected() {
    for intent in [
        velocity(3.1, 0.0, 0.0, 0.0),
        velocity(0.0, -3.1, 0.0, 0.0),
        velocity(0.0, 0.0, 1.6, 0.0),
        velocity(0.0, 0.0, 0.0, 1.0),
    ] {
        let mut frame = base_frame();
        frame.intent = Some(intent);
        assert_eq!(
            gate_frame(&frame, &motion_scope()),
            Err(FrameRejectionReason::LimitExceeded),
            "{intent:?}"
        );
    }
}

#[test]
fn an_unadvertised_action_is_rejected() {
    let mut frame = base_frame();
    frame.actions = vec![ControlAction::GimbalRecenter];
    assert_eq!(
        gate_frame(&frame, &motion_scope()),
        Err(FrameRejectionReason::UnsupportedAction)
    );
}

#[test]
fn an_unadvertised_mode_target_is_rejected() {
    let mut frame = base_frame();
    frame.actions = vec![ControlAction::ModeRequest {
        target: ModeTarget::Return,
    }];
    assert_eq!(
        gate_frame(&frame, &motion_scope()),
        Err(FrameRejectionReason::UnsupportedAction)
    );
    frame.actions = vec![ControlAction::ModeRequest {
        target: ModeTarget::CameraVelocity,
    }];
    assert!(gate_frame(&frame, &motion_scope()).is_ok());
}

#[test]
fn repeated_and_conflicting_action_sets_are_rejected_whole() {
    let mut frame = base_frame();
    frame.actions = vec![ControlAction::Arm, ControlAction::Arm];
    assert_eq!(
        gate_frame(&frame, &motion_scope()),
        Err(FrameRejectionReason::ConflictingActions)
    );
    frame.actions = vec![ControlAction::Arm, ControlAction::Disarm];
    assert_eq!(
        gate_frame(&frame, &motion_scope()),
        Err(FrameRejectionReason::ConflictingActions)
    );
}

/// The single legacy boundary: sticks scale by the SAME advertised limits a
/// typed client scales by, mapped buttons become typed actions, and an
/// unmapped button drops instead of leaking a number downstream.
#[test]
fn a_legacy_payload_translates_through_the_declared_map() {
    let mut frame = base_frame();
    frame.payload = ControlPayload {
        axes: vec![
            (LogicalAxisId::new(0), 0.5),
            (LogicalAxisId::new(1), -1.0),
            (LogicalAxisId::new(2), 1.0),
            (LogicalAxisId::new(3), 0.5),
        ],
        edges: vec![
            (LogicalButtonId::new(0), ButtonEdge::Pressed),
            (LogicalButtonId::new(7), ButtonEdge::Pressed),
            (LogicalButtonId::new(1), ButtonEdge::Released),
        ],
    };
    let gated = gate_frame(&frame, &motion_scope()).expect("translates");
    assert!(!gated.carries_payload(), "no numeric payload survives");
    let Some(ControlIntent::Velocity(v)) = gated.intent else {
        panic!("expected a velocity intent");
    };
    assert_eq!(v.frame, ReferenceFrame::BodyFrd);
    assert_eq!(v.vx, -3.0, "pitch -1.0 x 3.0");
    assert_eq!(v.vy, 1.5, "roll 0.5 x 3.0");
    assert_eq!(v.vz, -1.5, "throttle 1.0, sign -1, x 1.5");
    assert_eq!(v.yaw_rate, 0.45, "yaw 0.5 x 0.9");
    assert_eq!(
        gated.actions,
        vec![ControlAction::Arm],
        "mapped pressed edge becomes Arm; unmapped button 7 drops; a \
         released edge is not a press"
    );
}

#[test]
fn a_legacy_payload_on_a_scope_without_a_map_is_rejected() {
    let mut scope = motion_scope();
    scope.legacy = None;
    let mut frame = base_frame();
    frame.payload.axes = vec![(LogicalAxisId::new(1), 0.5)];
    assert_eq!(
        gate_frame(&frame, &scope),
        Err(FrameRejectionReason::UnsupportedIntent)
    );
}

#[test]
fn a_legacy_gimbal_frame_translates_rates_and_recenter() {
    let mut frame = base_frame();
    frame.scope = ScopeId::new("vehicle.gimbal");
    frame.payload = ControlPayload {
        axes: vec![(LogicalAxisId::new(1), -1.0), (LogicalAxisId::new(3), 0.5)],
        edges: vec![(LogicalButtonId::new(0), ButtonEdge::Pressed)],
    };
    let gated = gate_frame(&frame, &gimbal_scope()).expect("translates");
    let Some(ControlIntent::GimbalRate(rate)) = gated.intent else {
        panic!("expected a gimbal-rate intent");
    };
    assert_eq!(rate.pitch_rate, -0.8);
    assert_eq!(rate.yaw_rate, 0.4);
    assert_eq!(gated.actions, vec![ControlAction::GimbalRecenter]);

    // A recenter-only legacy frame carries the action with no rate intent.
    let mut recenter_only = base_frame();
    recenter_only.scope = ScopeId::new("vehicle.gimbal");
    recenter_only.payload.edges = vec![(LogicalButtonId::new(0), ButtonEdge::Pressed)];
    let gated = gate_frame(&recenter_only, &gimbal_scope()).expect("translates");
    assert_eq!(gated.intent, None);
    assert_eq!(gated.actions, vec![ControlAction::GimbalRecenter]);
}

/// Translation cannot exceed what a typed client could send: an out-of-range
/// legacy stick clamps to full scale BEFORE limit scaling.
#[test]
fn an_out_of_range_legacy_stick_clamps_to_full_scale() {
    let mut frame = base_frame();
    frame.payload.axes = vec![
        (LogicalAxisId::new(0), 0.0),
        (LogicalAxisId::new(1), 5.0),
        (LogicalAxisId::new(2), 0.0),
        (LogicalAxisId::new(3), 0.0),
    ];
    let gated = gate_frame(&frame, &motion_scope()).expect("translates");
    let Some(ControlIntent::Velocity(v)) = gated.intent else {
        panic!("expected a velocity intent");
    };
    assert_eq!(v.vx, 3.0);
}

/// The typed translation is total, so partial legacy coverage is rejected —
/// an omitted axis must never silently become an explicit neutral (which
/// could demonstrate a neutral activation the sender never proved).
#[test]
fn a_partial_legacy_payload_is_rejected() {
    let mut frame = base_frame();
    frame.payload.axes = vec![(LogicalAxisId::new(1), 0.0)];
    assert_eq!(
        gate_frame(&frame, &motion_scope()),
        Err(FrameRejectionReason::PartialCommand)
    );
}
