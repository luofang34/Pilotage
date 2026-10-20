use pilotage_client_session::ScopeCatalog;

use super::*;

fn vehicle() -> VehicleCatalog {
    let unsupported = wire::PayloadSupportState::Unsupported as i32;
    let mut vehicle = VehicleCatalog {
        vehicle_id: 1,
        display_name: "Camera vehicle".into(),
        vehicle_class: Some(pilotage_protocol::VehicleClass::Multicopter),
        heading_reference: None,
        scopes: Vec::new(),
    };
    vehicle.scopes.push(ScopeCatalog {
        scope: GIMBAL_SCOPE.into(),
        authority_group: GIMBAL_SCOPE.into(),
        command_only: false,
        actions: Vec::new(),
        intents: vec![wire::IntentCapability {
            family: wire::IntentFamily::GimbalRate as i32,
            ..Default::default()
        }],
        payload: Some(wire::PayloadCapabilities {
            gimbal: Some(wire::PayloadGimbalCapabilities {
                pitch: Some(wire::PayloadAxisCapabilities {
                    rate: Some(rate(wire::PayloadSupportState::Available)),
                    ..Default::default()
                }),
                yaw: Some(wire::PayloadAxisCapabilities {
                    position: Some(wire::PayloadPositionCapability {
                        support: unsupported,
                        ..Default::default()
                    }),
                    rate: Some(rate(wire::PayloadSupportState::Unsupported)),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
    });
    vehicle.scopes.push(ScopeCatalog {
        scope: pilotage_protocol::MOTION_SCOPE.into(),
        authority_group: pilotage_protocol::MOTION_SCOPE.into(),
        command_only: false,
        payload: None,
        actions: Vec::new(),
        intents: vec![wire::IntentCapability {
            family: wire::IntentFamily::Velocity as i32,
            frames: vec![wire::ReferenceFrame::BodyFrd as i32],
            max_angular: 1.0,
            ..Default::default()
        }],
    });
    vehicle
}

fn rate(support: wire::PayloadSupportState) -> wire::PayloadRateCapability {
    wire::PayloadRateCapability {
        support: support as i32,
        max_rate_rad_s: Some(1.0),
        ..Default::default()
    }
}

fn gimbal(vehicle: &mut VehicleCatalog) -> &mut wire::PayloadGimbalCapabilities {
    vehicle.scopes[0]
        .payload
        .as_mut()
        .unwrap()
        .gimbal
        .as_mut()
        .unwrap()
}

#[test]
fn only_explicit_pitch_only_control_uses_the_airframe() {
    let mut vehicle = vehicle();
    assert_eq!(
        camera_pan_routing(&vehicle, Mode::QuadPilot),
        CameraPanRouting::AirframeYaw
    );
    for support in [
        wire::PayloadSupportState::Unspecified,
        wire::PayloadSupportState::Unavailable,
    ] {
        gimbal(&mut vehicle).yaw.as_mut().unwrap().rate = Some(rate(support));
        assert_eq!(
            camera_pan_routing(&vehicle, Mode::QuadPilot),
            CameraPanRouting::Unavailable
        );
    }
    gimbal(&mut vehicle).yaw.as_mut().unwrap().rate =
        Some(rate(wire::PayloadSupportState::Available));
    assert_eq!(
        camera_pan_routing(&vehicle, Mode::QuadPilot),
        CameraPanRouting::GimbalYaw
    );
    gimbal(&mut vehicle).yaw = None;
    assert_eq!(
        camera_pan_routing(&vehicle, Mode::QuadPilot),
        CameraPanRouting::Unavailable
    );
    vehicle.scopes[0].payload = None;
    assert_eq!(
        camera_pan_routing(&vehicle, Mode::QuadPilot),
        CameraPanRouting::GimbalYaw
    );
}

#[test]
fn position_pan_and_unknown_position_support_do_not_select_airframe_yaw() {
    for support in [
        wire::PayloadSupportState::Available,
        wire::PayloadSupportState::Unavailable,
        wire::PayloadSupportState::Unspecified,
    ] {
        let mut vehicle = vehicle();
        gimbal(&mut vehicle)
            .yaw
            .as_mut()
            .unwrap()
            .position
            .as_mut()
            .unwrap()
            .support = support as i32;
        assert_eq!(
            camera_pan_routing(&vehicle, Mode::QuadPilot),
            CameraPanRouting::Unavailable
        );
    }
    let mut vehicle = vehicle();
    gimbal(&mut vehicle).pitch.as_mut().unwrap().rate =
        Some(rate(wire::PayloadSupportState::Unavailable));
    assert_eq!(
        camera_pan_routing(&vehicle, Mode::QuadPilot),
        CameraPanRouting::Unavailable
    );
}

#[test]
fn discrete_flight_and_invalid_motion_envelopes_never_offer_airframe_yaw() {
    let mut vehicle = vehicle();
    vehicle.scopes[1].command_only = true;
    assert_eq!(
        camera_pan_routing(&vehicle, Mode::QuadPilot),
        CameraPanRouting::AirframeYawUnavailable
    );
    vehicle.scopes[1].command_only = false;
    for limit in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        vehicle.scopes[1].intents[0].max_angular = limit;
        assert_eq!(
            camera_pan_routing(&vehicle, Mode::QuadPilot),
            CameraPanRouting::AirframeYawUnavailable
        );
    }
    vehicle.scopes[1].intents[0].max_angular = 1.0;
    vehicle.scopes[1].intents[0].frames.clear();
    assert_eq!(
        camera_pan_routing(&vehicle, Mode::QuadPilot),
        CameraPanRouting::AirframeYawUnavailable
    );
    vehicle.scopes[1].intents.clear();
    assert_eq!(
        camera_pan_routing(&vehicle, Mode::QuadPilot),
        CameraPanRouting::AirframeYawUnavailable
    );
}

#[test]
fn the_selected_scheme_and_its_frames_bound_the_fallback() {
    let mut vehicle = vehicle();
    assert_eq!(
        camera_pan_routing(&vehicle, Mode::QuadCruise),
        CameraPanRouting::AirframeYaw
    );
    assert_eq!(
        camera_pan_routing(&vehicle, Mode::Fpv),
        CameraPanRouting::AirframeYawUnavailable
    );
    vehicle.scopes[1].scope = pilotage_protocol::DIRECT_SCOPE.into();
    vehicle.scopes[1].intents[0] = wire::IntentCapability {
        family: wire::IntentFamily::AttitudeThrust as i32,
        frames: vec![wire::ReferenceFrame::LocalNed as i32],
        max_yaw_rate: 1.0,
        ..Default::default()
    };
    assert_eq!(
        camera_pan_routing(&vehicle, Mode::Fpv),
        CameraPanRouting::AirframeYaw
    );
    vehicle.scopes[1].intents[0].frames.clear();
    assert_eq!(
        camera_pan_routing(&vehicle, Mode::Fpv),
        CameraPanRouting::AirframeYawUnavailable
    );
    vehicle.scopes[1].intents[0].family = wire::IntentFamily::BodyRate as i32;
    assert_eq!(
        camera_pan_routing(&vehicle, Mode::Acro),
        CameraPanRouting::AirframeYaw
    );
    for mode in [Mode::Plane, Mode::Rover] {
        assert_eq!(
            camera_pan_routing(&vehicle, mode),
            CameraPanRouting::Unavailable
        );
    }
}
