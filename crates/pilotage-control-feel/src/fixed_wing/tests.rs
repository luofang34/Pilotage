use super::{FixedWingEnvelope, ValidatedFixedWingEnvelope};
use crate::ValidationError;

#[test]
fn the_c172_envelope_validates_and_turns_as_a_coordinated_turn_does() {
    let envelope = ValidatedFixedWingEnvelope::new(FixedWingEnvelope::C172).expect("valid");
    let c172 = envelope.envelope();
    assert!((c172.route_bank_rad.to_degrees() - 25.0).abs() < 1e-9);
    // g·tan(25°)/55 m/s.
    assert!((c172.turn_rate_rps(55.0) - 0.083_09).abs() < 1e-4);
    assert!(c172.max_turn_rate_rps() > c172.turn_rate_rps(c172.cruise_airspeed_mps));
}

#[test]
fn a_cruise_outside_the_airspeed_range_is_refused() {
    let slow = FixedWingEnvelope {
        cruise_airspeed_mps: 30.0,
        ..FixedWingEnvelope::C172
    };
    assert_eq!(
        ValidatedFixedWingEnvelope::new(slow),
        Err(ValidationError::InvalidOrder {
            lower: "fixed_wing.min_airspeed_mps",
            upper: "fixed_wing.cruise_airspeed_mps",
        })
    );
    let fast = FixedWingEnvelope {
        cruise_airspeed_mps: 70.0,
        ..FixedWingEnvelope::C172
    };
    assert!(ValidatedFixedWingEnvelope::new(fast).is_err());
}

#[test]
fn a_value_that_is_not_a_positive_number_or_a_steep_bank_is_refused() {
    for envelope in [
        FixedWingEnvelope {
            max_sink_mps: 0.0,
            ..FixedWingEnvelope::C172
        },
        FixedWingEnvelope {
            max_climb_mps: f64::NAN,
            ..FixedWingEnvelope::C172
        },
        FixedWingEnvelope {
            route_bank_rad: 1.2,
            ..FixedWingEnvelope::C172
        },
    ] {
        assert!(matches!(
            ValidatedFixedWingEnvelope::new(envelope),
            Err(ValidationError::FieldOutOfRange { .. })
        ));
    }
}

#[test]
fn the_envelope_reads_from_strict_json() {
    let json = r#"{"cruise_airspeed_mps":55.0,"min_airspeed_mps":33.0,"max_airspeed_mps":65.0,
        "max_climb_mps":2.5,"max_sink_mps":5.0,"route_bank_rad":0.4363323129985824}"#;
    let parsed: FixedWingEnvelope = serde_json::from_str(json).expect("parses");
    assert_eq!(parsed, FixedWingEnvelope::C172);
    let extra = json.replace("}", r#","hover_thrust":0.5}"#);
    assert!(serde_json::from_str::<FixedWingEnvelope>(&extra).is_err());
}
