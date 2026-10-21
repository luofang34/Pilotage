use super::*;
use turret_core::*;

#[test]
fn physical_support_and_unimplemented_operations_keep_distinct_wire_declarations() {
    let position = PositionCapability {
        implementation: Implementation::Native,
        range: AngleRange {
            minimum: Angle::radians(-1.0).unwrap(),
            maximum: Angle::ZERO,
        },
        frames: Frames {
            mount: true,
            ..Frames::default()
        },
        maximum_rate: None,
    };
    let pitch = AxisCapabilities {
        mechanics: AxisMechanics {
            movement: HardwareSupport::Supported,
            stabilization: HardwareSupport::Supported,
        },
        position: Capability::Unimplemented {
            capability: Some(position),
        },
        rate: Capability::Unimplemented { capability: None },
        ..AxisCapabilities::UNKNOWN
    };
    let caps = PayloadCapabilities {
        revision: 3,
        gimbal: GimbalCapabilities {
            axes: Axes {
                pitch,
                roll: AxisCapabilities::UNSUPPORTED,
                yaw: AxisCapabilities::UNKNOWN,
            },
            recenter: Capability::Unknown,
            stop: StopGuarantee::Unknown,
        },
        zoom: Capability::Unimplemented { capability: None },
    };
    let declared = payload_capabilities_to_wire("camera", &caps, &["gimbal"]);
    let gimbal = declared.gimbal.unwrap();
    let pitch = gimbal.pitch.unwrap();
    assert!(pilotage_protocol::payload_rate_limit(Some(&pitch)).is_none());
    let mechanics = pitch.mechanics.unwrap();
    assert_eq!(
        mechanics.movement,
        wire::PayloadHardwareSupport::Supported as i32
    );
    assert_eq!(
        mechanics.stabilization,
        wire::PayloadHardwareSupport::Supported as i32
    );
    let position = pitch.position.unwrap();
    assert_eq!(
        position.support,
        wire::PayloadSupportState::Unimplemented as i32
    );
    assert_eq!(position.minimum_rad, Some(-1.0));
    assert_eq!(position.maximum_rad, Some(0.0));
    assert_eq!(position.frames, [wire::PayloadReferenceFrame::Mount as i32]);
    let rate = pitch.rate.unwrap();
    assert_eq!(
        rate.support,
        wire::PayloadSupportState::Unimplemented as i32
    );
    assert!(rate.max_rate_rad_s.is_none());
    let roll = gimbal.roll.unwrap();
    let mechanics = roll.mechanics.unwrap();
    assert_eq!((mechanics.movement, mechanics.stabilization), (0, 0));
    assert_eq!(
        roll.position.unwrap().support,
        wire::PayloadSupportState::Unsupported as i32
    );
    assert_eq!(
        declared.zoom.unwrap().support,
        wire::PayloadSupportState::Unimplemented as i32
    );
}
