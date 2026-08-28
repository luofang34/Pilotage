//! Publication of simulator-neutral tuning campaign evidence.
//!
//! SIM / NOT FOR FLIGHT.

#![forbid(unsafe_code)]

mod bench;
mod error;
mod publish;
mod scoring;
mod vehicle_policy;

pub use bench::{
    BenchBackend, BenchGates, BenchHandle, BenchVehicle, BenchVehicleAdapter, BenchVehicleFactory,
    bench_scenario, bench_stage, parameter, warm_start_parameters,
};
pub use error::CampaignError;
pub use publish::publish_journal_evidence_blocking;
pub use scoring::{FlightQualityEvaluator, channel};
pub use vehicle_policy::{
    alia250_promotion_policy, alia250_qualification_policy, alia250_required_policy,
    x500_promotion_policy, x500_qualification_policy, x500_required_policy,
};
