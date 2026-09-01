#![allow(clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn the_nominal_plant_adds_no_mass_and_measures_its_own_ratio() {
    let value = PlantCondition::nominal();

    value.validate().expect("nominal plant");
    assert!((value.payload_mass_delta_kg - 0.0).abs() < f64::EPSILON);
    assert!((value.longitudinal_cg_offset_m - 0.0).abs() < f64::EPSILON);
    assert!((value.lateral_cg_offset_m - 0.0).abs() < f64::EPSILON);
    assert_eq!(
        value.hover_thrust_expectation,
        HoverThrustExpectation::MeasuredWeightRatio
    );
    assert_eq!(value.hover_thrust_expectation.explicit(), None);
}

#[test]
fn each_plant_request_holds_its_closed_bound() {
    for delta in [-0.1, 2_000.1, f64::NAN, f64::INFINITY] {
        let value = PlantCondition {
            payload_mass_delta_kg: delta,
            ..PlantCondition::nominal()
        };
        assert!(value.validate().is_err(), "payload delta {delta} passed");
    }
    for offset in [-2.1, 2.1, f64::NAN] {
        let longitudinal = PlantCondition {
            longitudinal_cg_offset_m: offset,
            ..PlantCondition::nominal()
        };
        let lateral = PlantCondition {
            lateral_cg_offset_m: offset,
            ..PlantCondition::nominal()
        };
        assert!(longitudinal.validate().is_err(), "longitudinal {offset}");
        assert!(lateral.validate().is_err(), "lateral {offset}");
    }
    for value in [
        PlantCondition {
            payload_mass_delta_kg: 2_000.0,
            longitudinal_cg_offset_m: -2.0,
            lateral_cg_offset_m: 2.0,
            hover_thrust_expectation: HoverThrustExpectation::MeasuredWeightRatio,
        },
        PlantCondition {
            payload_mass_delta_kg: 0.0,
            longitudinal_cg_offset_m: 0.0,
            lateral_cg_offset_m: 0.0,
            hover_thrust_expectation: HoverThrustExpectation::ExplicitRatio {
                ratio: 0.5,
                maximum_error: 0.1,
            },
        },
    ] {
        value.validate().expect("bounded plant");
    }
}

#[test]
fn an_explicit_hover_ratio_holds_its_own_bound() {
    for (ratio, maximum_error) in [(0.49, 0.05), (1.51, 0.05), (1.0, -0.01), (1.0, 0.11)] {
        let value = PlantCondition {
            hover_thrust_expectation: HoverThrustExpectation::ExplicitRatio {
                ratio,
                maximum_error,
            },
            ..PlantCondition::nominal()
        };
        assert!(
            matches!(value.validate(), Err(ValidationError::OutOfRange { .. })),
            "ratio {ratio} error {maximum_error} passed"
        );
    }
}

#[test]
fn a_measured_ratio_check_refuses_an_absent_readback() {
    let measured = HoverThrustExpectation::MeasuredWeightRatio;

    assert!(measured.accepts(0.7));
    assert!(measured.accepts(1.3));
    assert!(!measured.accepts(0.0));
    assert!(!measured.accepts(-1.0));
    assert!(!measured.accepts(f64::NAN));
    assert!(!measured.accepts(f64::INFINITY));
}

#[test]
fn an_explicit_ratio_check_accepts_its_closed_error_band() {
    let value = HoverThrustExpectation::ExplicitRatio {
        ratio: 1.0,
        maximum_error: 0.05,
    };

    assert!(value.accepts(1.0));
    assert!(value.accepts(1.05));
    assert!(value.accepts(0.95));
    assert!(!value.accepts(1.06));
    assert!(!value.accepts(0.94));
    assert!(!value.accepts(f64::NAN));
    assert_eq!(value.explicit(), Some((1.0, 0.05)));
}

#[test]
fn a_plant_document_refuses_an_unknown_field() {
    let known = serde_json::json!({
        "payload_mass_delta_kg": 1.0,
        "longitudinal_cg_offset_m": 0.1,
        "lateral_cg_offset_m": -0.1,
        "hover_thrust_expectation": {"kind": "measured_weight_ratio"}
    });
    let value =
        serde_json::from_value::<PlantCondition>(known.clone()).expect("known plant fields");
    value.validate().expect("valid plant");

    let mut unknown = known;
    unknown["unexpected"] = serde_json::Value::Bool(true);
    assert!(serde_json::from_value::<PlantCondition>(unknown).is_err());

    let unknown_expectation = serde_json::json!({
        "kind": "explicit_ratio",
        "ratio": 1.0,
        "maximum_error": 0.01,
        "unexpected": true
    });
    assert!(serde_json::from_value::<HoverThrustExpectation>(unknown_expectation).is_err());
}
