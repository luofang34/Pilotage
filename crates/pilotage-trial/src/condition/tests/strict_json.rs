#![allow(clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn a_noise_request_needs_an_amplitude_and_a_sample_interval() {
    let lane = serde_json::json!({
        "sensor": "accelerometer",
        "axis": "x",
        "peak_amplitude_mps2": 0.1,
        "update_interval_samples": 10
    });
    for field in ["peak_amplitude_mps2", "update_interval_samples", "axis"] {
        let mut missing = lane.clone();
        missing.as_object_mut().expect("sensor lane").remove(field);
        let sensor = serde_json::json!({"kind": "bounded_noise", "lanes": [missing]});
        assert!(serde_json::from_value::<SensorCondition>(sensor).is_err());
    }
}

#[test]
fn an_unknown_perturbation_field_is_refused_at_every_depth() {
    let sensor = serde_json::json!({
        "kind": "bounded_noise",
        "lanes": [{
            "sensor": "absolute_pressure",
            "peak_amplitude_hpa": 1.0,
            "update_interval_samples": 10,
            "unexpected": true
        }]
    });
    assert!(serde_json::from_value::<SensorCondition>(sensor).is_err());

    let actuator = serde_json::json!({
        "authority_scale_basis_points": 10_000,
        "command_loss": {"kind": "none", "unexpected": true}
    });
    assert!(serde_json::from_value::<ActuatorCondition>(actuator).is_err());

    let initialization = serde_json::json!({
        "hover_thrust_force": {
            "kind": "scale_baseline",
            "scale_basis_points": 10_000,
            "unexpected": true
        }
    });
    assert!(serde_json::from_value::<ControllerInitializationCondition>(initialization).is_err());
}

#[test]
fn a_boolean_declaration_cannot_stand_for_a_sensor_request() {
    let mut json = serde_json::to_value(condition(42)).expect("condition value");
    json["sensor"] = serde_json::Value::Bool(true);
    assert!(ConditionSet::from_json(&serde_json::to_vec(&json).expect("boolean JSON")).is_err());

    json["sensor"] = serde_json::json!({"kind": "bounded_noise"});
    assert!(ConditionSet::from_json(&serde_json::to_vec(&json).expect("lane-free JSON")).is_err());
}
