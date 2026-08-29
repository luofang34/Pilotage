//! The canonical values and signals the vehicle port is answerable for.

use flight_tune::{CanonicalTelemetryKey, ControlChannel, ObservedSignal, SignalSelector};

use crate::runtime::quality::{TelemetrySource, source_of, vehicle_supplied};
use crate::runtime::telemetry::{VehicleSignals, require_vehicle_states};

use super::frame;

fn commanding(normalized: f64) -> VehicleSignals {
    VehicleSignals {
        normalized_command: Some(normalized),
        channel: Some(ControlChannel::Pitch),
        transmitted_attitude_rad: Some(0.1),
        saturated: normalized.abs() >= 1.0,
        link_valid: true,
        estimator_valid: true,
    }
}

#[test]
fn a_commanding_frame_states_its_channel_and_its_transmitted_setpoint() {
    let signals = commanding(0.4).observed().expect("the observed signals");
    assert_eq!(signals.len(), 2);
    assert!(signals.contains(&ObservedSignal {
        selector: SignalSelector::NormalizedControl {
            channel: ControlChannel::Pitch,
        },
        value: 0.4,
    }));
}

#[test]
fn a_frame_that_commands_nothing_states_no_control_signal() {
    let signals = VehicleSignals::default()
        .observed()
        .expect("the observed signals");
    assert!(signals.is_empty());
}

#[test]
fn a_commanded_value_that_is_not_a_number_is_refused() {
    let mut signals = commanding(0.4);
    signals.normalized_command = Some(f64::NAN);
    signals
        .observed()
        .expect_err("a commanded value that is not a number must be refused");
    signals
        .canonical_values(false)
        .expect_err("a commanded value that is not a number must be refused");
}

#[test]
fn the_canonical_values_are_exactly_the_fields_this_port_supplies() {
    let values = commanding(1.0)
        .canonical_values(true)
        .expect("the canonical values");
    let names = vehicle_supplied()
        .iter()
        .map(|key| key.as_str().to_owned())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        values
            .keys()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        names
    );
    assert!((values[CanonicalTelemetryKey::ActuatorSaturated.as_str()] - 1.0).abs() < f64::EPSILON);
    assert!((values[CanonicalTelemetryKey::Recovered.as_str()] - 1.0).abs() < f64::EPSILON);
    assert!(values[CanonicalTelemetryKey::CommandLinkValid.as_str()] > 0.5);
}

#[test]
fn every_canonical_field_names_one_side_of_the_run() {
    for key in CanonicalTelemetryKey::ALL {
        let expected = if vehicle_supplied().contains(&key) {
            TelemetrySource::Vehicle
        } else {
            TelemetrySource::SimulatorTruth
        };
        assert_eq!(source_of(key), expected, "{}", key.as_str());
    }
    assert_eq!(
        source_of(CanonicalTelemetryKey::PositionErrorM),
        TelemetrySource::SimulatorTruth
    );
    assert_eq!(
        source_of(CanonicalTelemetryKey::CommandPrimary),
        TelemetrySource::Vehicle
    );
}

#[test]
fn a_frame_that_omits_a_vehicle_state_is_refused_by_name() {
    let exact = frame(1, 1_000);
    assert_eq!(
        require_vehicle_states(&exact).expect("both states"),
        (true, true)
    );

    let mut without_link = exact.clone();
    without_link.link_valid = None;
    let detail = require_vehicle_states(&without_link)
        .expect_err("a frame with no link state must be refused")
        .to_string();
    assert!(detail.contains("control-link validity"), "{detail}");

    let mut without_estimator = exact;
    without_estimator.estimator_valid = None;
    let detail = require_vehicle_states(&without_estimator)
        .expect_err("a frame with no estimator state must be refused")
        .to_string();
    assert!(detail.contains("estimator validity"), "{detail}");
}
