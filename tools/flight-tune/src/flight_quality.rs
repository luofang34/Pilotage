//! Canonical streaming flight-quality evaluation.

mod config;
mod gates;
mod metrics;
mod telemetry;

pub use config::{
    FlightQualityGate, FlightQualityGateConfig, FlightQualityMetricConfig, FlightQualityScales,
    FlightQualityScenario, FlightQualityWeights, ReleasePlan, StepPlan, WindPlan,
};
pub use gates::FlightQualityGateEvaluator;
pub use metrics::{FlightQualityMetricEvaluator, FlightQualityReport};
pub use telemetry::CanonicalTelemetryKey;

#[cfg(test)]
#[path = "flight_quality/tests.rs"]
mod tests;
