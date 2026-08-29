//! Canonical streaming flight-quality evaluation.

mod config;
mod gates;
mod identity;
mod metrics;
mod telemetry;

pub use config::{
    FlightQualityGate, FlightQualityGateConfig, FlightQualityMetricConfig, FlightQualityScales,
    FlightQualityScenario, FlightQualityWeights, MANDATORY_CRASH_GATE_ID, ReleasePlan, StepPlan,
    WindPlan,
};
pub use gates::FlightQualityGateEvaluator;
pub use identity::{
    EVALUATOR_IMPLEMENTATION_SCHEMA_VERSION, GATE_IMPLEMENTATION_ID, METRIC_IMPLEMENTATION_ID,
};
pub use metrics::{FlightQualityMetricEvaluator, FlightQualityReport};
pub use telemetry::CanonicalTelemetryKey;

#[cfg(test)]
#[path = "flight_quality/tests.rs"]
mod tests;
