//! Publication of simulator-neutral tuning campaign evidence.
//!
//! SIM / NOT FOR FLIGHT.

#![forbid(unsafe_code)]

mod bench;
mod error;
mod publish;
mod scenario;
mod scoring;
mod vehicle_policy;

pub use bench::{
    BENCH_FINAL_TRIAL_ID, BENCH_PROMOTION_TRIAL_ID, BenchBackend, BenchGates, BenchHandle,
    BenchVehicle, BenchVehicleAdapter, BenchVehicleBindingRollback, BenchVehicleFactory,
    bench_mission_revision_id, bench_physical_target, bench_response_targets, bench_scenario,
    bench_stage, parameter, warm_start_parameters,
};
pub use error::CampaignError;
pub use publish::publish_journal_evidence_blocking;
pub use scenario::{
    ALIA250_MATRIX, LoadedCell, LoadedMatrix, MatrixCell, MatrixCondition, MatrixPartition,
    MatrixReport, MatrixStimulus, ScenarioMatrix, UncertaintyFactor,
    alia250_matrix_response_targets, condition_path, matrix_mission, scenario_path,
};
pub use scoring::{FlightQualityEvaluator, channel};
pub use vehicle_policy::{
    alia250_promotion_policy, alia250_qualification_policy, alia250_required_policy,
    alia250_response_targets, x500_promotion_policy, x500_qualification_policy,
    x500_required_policy, x500_response_targets,
};
