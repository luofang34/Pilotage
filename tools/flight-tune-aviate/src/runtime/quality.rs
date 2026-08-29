//! The canonical telemetry the Aviate runtime is answerable for.
//!
//! The scoring layer names its fields once, in the simulator-neutral
//! harness. This module states which of those fields the Aviate vehicle
//! port supplies and which the simulator truth projection supplies, so a
//! run that is missing a scored field fails on the missing name rather
//! than on a silently absent value.

use flight_tune::CanonicalTelemetryKey;

/// Which side of the run supplies one canonical telemetry field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TelemetrySource {
    /// The vehicle action port supplies the value.
    Vehicle,
    /// The simulator truth projection supplies the value.
    SimulatorTruth,
}

/// The fields the Aviate vehicle port is answerable for.
///
/// Every other canonical field comes from the simulator truth projection.
const VEHICLE_SUPPLIED: [CanonicalTelemetryKey; 6] = [
    CanonicalTelemetryKey::ActuatorEffort,
    CanonicalTelemetryKey::ActuatorSaturated,
    CanonicalTelemetryKey::CommandLinkValid,
    CanonicalTelemetryKey::CommandPrimary,
    CanonicalTelemetryKey::EstimatorValid,
    CanonicalTelemetryKey::Recovered,
];

/// Which side of one run supplies a canonical telemetry field.
#[must_use]
pub fn source_of(key: CanonicalTelemetryKey) -> TelemetrySource {
    if VEHICLE_SUPPLIED.contains(&key) {
        TelemetrySource::Vehicle
    } else {
        TelemetrySource::SimulatorTruth
    }
}

/// Every canonical field the Aviate vehicle port supplies.
#[must_use]
pub const fn vehicle_supplied() -> &'static [CanonicalTelemetryKey] {
    &VEHICLE_SUPPLIED
}

/// The Boolean encoding the canonical telemetry contract uses.
#[must_use]
pub const fn boolean(value: bool) -> f64 {
    if value { 1.0 } else { 0.0 }
}
