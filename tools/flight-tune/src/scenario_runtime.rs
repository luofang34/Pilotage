//! Simulator-neutral mission runtime contracts for tuning campaigns.

mod document;
mod frame;
mod host;
mod port;
mod uncertainty;

pub use document::{
    calibration_mission_document, mission_document_from_scenario, reference_observation_scenario,
};
pub use frame::{KinematicTruth, ScenarioFrame};
pub use host::CampaignMissionRuntime;
pub use port::{
    ScenarioObservationReceipt, ScenarioRuntime, ScenarioRuntimeError, ScenarioStopContext,
    ScenarioStopReason,
};
pub use uncertainty::{ConditionAdmission, UncertaintyDeclaration};

#[cfg(test)]
mod tests;
