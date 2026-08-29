#![allow(clippy::expect_used, clippy::panic)]

use super::*;

fn scaled(scale_basis_points: u16) -> ControllerInitializationCondition {
    ControllerInitializationCondition {
        hover_thrust_force: HoverThrustForceInitialization::ScaleBaseline { scale_basis_points },
    }
}

#[test]
fn the_hover_force_scale_holds_the_closed_basis_point_bound() {
    for scale in [7_999, 12_001] {
        assert!(matches!(
            scaled(scale).validate(),
            Err(ValidationError::OutOfRange { .. })
        ));
    }
    for scale in [8_000, 10_000, 12_000] {
        scaled(scale).validate().expect("bounded hover scale");
    }
    assert!(ControllerInitializationCondition::nominal().has_nominal_hover_thrust_force());
    assert!(!scaled(9_000).has_nominal_hover_thrust_force());
}

#[test]
fn only_a_non_nominal_hover_force_needs_a_capability() {
    assert!(
        ControllerInitializationCondition::nominal()
            .required_capabilities()
            .is_empty()
    );
    assert_eq!(
        scaled(9_000).required_capabilities(),
        vec![BackendCapability::HoverTrimUncertainty]
    );
}

#[test]
fn the_scale_acts_in_the_force_domain_inside_an_open_valid_interval() {
    let scale = HoverThrustForceInitialization::ScaleBaseline {
        scale_basis_points: 9_000,
    };

    assert!(
        (scale
            .effective_force(1_000.0, 800.0, 1_100.0)
            .expect("effective force")
            - 900.0)
            .abs()
            < 1e-9
    );
    assert!(matches!(
        scale.effective_force(1_000.0, 900.0, 1_100.0),
        Err(ValidationError::InvalidRelation { .. })
    ));
    assert!(scale.effective_force(1_000.0, 0.0, 900.0).is_err());
    assert!(scale.effective_force(f64::NAN, 0.0, 1_100.0).is_err());
    assert!(scale.effective_force(1_000.0, 1_100.0, 800.0).is_err());
}

#[test]
fn json_rejects_an_unknown_kind_or_field() {
    let unknown_field = serde_json::json!({
        "hover_thrust_force": {
            "kind": "scale_baseline",
            "scale_basis_points": 10_000,
            "unexpected": true
        }
    });
    assert!(serde_json::from_value::<ControllerInitializationCondition>(unknown_field).is_err());

    let unknown_kind = serde_json::json!({
        "hover_thrust_force": {"kind": "absolute_force", "force_n": 12.0}
    });
    assert!(serde_json::from_value::<ControllerInitializationCondition>(unknown_kind).is_err());
}
