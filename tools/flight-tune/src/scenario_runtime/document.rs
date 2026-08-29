use serde::{Serialize, de::DeserializeOwned};

use pilotage_mission_core::{
    ExecutionPolicy, ExecutionTarget, FlightAction, MissionAction, MissionCapability,
    MissionCondition, MissionDocument, MissionPhase, NavigationDataIdentity, TrialAction,
};
use pilotage_trial::{
    BackendCapability, Comparison, Phase, PhaseAction, PhaseCondition, SCENARIO_SCHEMA_VERSION,
    Scenario,
};

/// Creates the declarative observation scenario used by a reference backend.
#[must_use]
pub fn reference_observation_scenario(id: &str, completion_time_ns: Option<u64>) -> Scenario {
    let exit_conditions = completion_time_ns.map_or_else(
        || vec![PhaseCondition::Always],
        |value_ns| {
            vec![PhaseCondition::SimulatorTime {
                comparison: Comparison::GreaterOrEqual,
                value_ns,
            }]
        },
    );
    Scenario {
        schema_version: SCENARIO_SCHEMA_VERSION,
        id: id.to_owned(),
        revision: 1,
        phases: vec![Phase {
            id: "observe".to_owned(),
            max_sim_time_ns: completion_time_ns
                .unwrap_or(1_000_000_000)
                .saturating_add(1_000_000_000),
            required_capabilities: vec![BackendCapability::SimulatorTime],
            entry_conditions: vec![PhaseCondition::Always],
            action: PhaseAction::Observe,
            exit_conditions,
            abort_conditions: Vec::new(),
        }],
    }
}

use super::ScenarioRuntimeError;

/// Projects one trial scenario into its calibration mission document.
///
/// The navigation-data identity names the authored trial scenario, so a
/// changed scenario changes the mission content digest.
///
/// # Errors
///
/// Returns an error when the trial scenario or projected mission is invalid.
pub fn calibration_mission_document(
    scenario: &Scenario,
    retry_limit: u16,
    receipt_timeout_ns: u64,
) -> Result<MissionDocument, ScenarioRuntimeError> {
    let source = scenario
        .canonical_digest()
        .map_err(|error| projection(error.to_string()))?;
    mission_document_from_scenario(
        scenario,
        NavigationDataIdentity {
            cycle: "calibration".to_owned(),
            snapshot_id: "trial-scenario".to_owned(),
            snapshot_digest: pilotage_mission_core::Digest::from_bytes(*source.as_bytes()),
        },
        retry_limit,
        receipt_timeout_ns,
    )
}

/// Projects one declarative trial scenario into the shared mission document.
///
/// # Errors
///
/// Returns an error when the trial scenario or projected mission is invalid.
pub fn mission_document_from_scenario(
    scenario: &Scenario,
    navigation_data_identity: NavigationDataIdentity,
    retry_limit: u16,
    receipt_timeout_ns: u64,
) -> Result<MissionDocument, ScenarioRuntimeError> {
    scenario
        .validate()
        .map_err(|error| projection(error.to_string()))?;
    let phases = scenario
        .phases
        .iter()
        .map(project_phase)
        .collect::<Result<Vec<_>, _>>()?;
    MissionDocument::new(
        format!("{}:{}", scenario.id, scenario.revision),
        navigation_data_identity,
        ExecutionPolicy {
            target: ExecutionTarget::Simulator,
            retry_limit,
            receipt_timeout_ns,
        },
        phases,
    )
    .map_err(|error| projection(error.to_string()))
}

fn project_phase(phase: &pilotage_trial::Phase) -> Result<MissionPhase, ScenarioRuntimeError> {
    let action = project_action(&phase.action)?;
    let mut required_capabilities: Vec<MissionCapability> =
        transcode(&phase.required_capabilities)?;
    if matches!(
        &action,
        MissionAction::Trial(TrialAction::ReleaseControl {} | TrialAction::Stop {})
    ) && !required_capabilities.contains(&MissionCapability::SimulatorControl)
    {
        required_capabilities.push(MissionCapability::SimulatorControl);
    }
    Ok(MissionPhase {
        id: phase.id.clone(),
        required_capabilities,
        entry_conditions: phase
            .entry_conditions
            .iter()
            .map(project_condition)
            .collect::<Result<Vec<_>, _>>()?,
        action,
        cleanup_actions: Vec::new(),
        completion_conditions: phase
            .exit_conditions
            .iter()
            .map(project_condition)
            .collect::<Result<Vec<_>, _>>()?,
        abort_conditions: phase
            .abort_conditions
            .iter()
            .map(project_condition)
            .collect::<Result<Vec<_>, _>>()?,
        simulator_time_deadline_ns: phase.max_sim_time_ns,
    })
}

fn project_condition(condition: &PhaseCondition) -> Result<MissionCondition, ScenarioRuntimeError> {
    match condition {
        PhaseCondition::Always => Ok(MissionCondition::Always {}),
        PhaseCondition::Lifecycle { state } => Ok(MissionCondition::Vehicle(
            pilotage_mission_core::VehicleCondition::Lifecycle {
                state: transcode(state)?,
            },
        )),
        PhaseCondition::GroundContact { expected } => Ok(MissionCondition::Vehicle(
            pilotage_mission_core::VehicleCondition::GroundContact {
                expected: *expected,
            },
        )),
        PhaseCondition::Crashed { expected } => Ok(MissionCondition::Vehicle(
            pilotage_mission_core::VehicleCondition::Crashed {
                expected: *expected,
            },
        )),
        PhaseCondition::LinkValid { expected } => Ok(MissionCondition::Vehicle(
            pilotage_mission_core::VehicleCondition::LinkValid {
                expected: *expected,
            },
        )),
        PhaseCondition::EstimatorValid { expected } => Ok(MissionCondition::Vehicle(
            pilotage_mission_core::VehicleCondition::EstimatorValid {
                expected: *expected,
            },
        )),
        PhaseCondition::SimulatorTime {
            comparison,
            value_ns,
        } => Ok(MissionCondition::Simulator(
            pilotage_mission_core::SimulatorCondition::Time {
                comparison: transcode(comparison)?,
                value_ns: *value_ns,
            },
        )),
        PhaseCondition::Signal {
            selector,
            comparison,
            value,
        } => Ok(MissionCondition::Signal(
            pilotage_mission_core::SignalCondition::Value {
                selector: transcode(selector)?,
                comparison: transcode(comparison)?,
                value: *value,
            },
        )),
    }
}

fn project_action(action: &PhaseAction) -> Result<MissionAction, ScenarioRuntimeError> {
    let action = match action {
        PhaseAction::Arm => return Ok(MissionAction::Flight(FlightAction::Arm {})),
        PhaseAction::Reset => TrialAction::Reset {},
        PhaseAction::WaitReady => TrialAction::WaitReady {},
        PhaseAction::ApplyConditions { condition_set } => TrialAction::ApplyConditions {
            condition_set: transcode(condition_set)?,
        },
        PhaseAction::ReachStartState { target } => TrialAction::ReachStartState {
            target: transcode(target)?,
        },
        PhaseAction::Settle => TrialAction::Settle {},
        PhaseAction::Stimulus {
            family,
            channel,
            mapping,
            envelope,
            waveform,
        } => TrialAction::Stimulate {
            family: transcode(family)?,
            channel: transcode(channel)?,
            mapping: transcode(mapping)?,
            envelope: transcode(envelope)?,
            waveform: transcode(waveform)?,
        },
        PhaseAction::ReleaseControl => TrialAction::ReleaseControl {},
        PhaseAction::Observe => TrialAction::Observe {},
        PhaseAction::Stop => TrialAction::Stop {},
        PhaseAction::Disarm => TrialAction::Disarm {},
        PhaseAction::CollectResults => TrialAction::CollectResults {},
    };
    Ok(MissionAction::Trial(action))
}

/// Projects one contract value into its mirror in the other crate.
///
/// The two crates hold the same shapes under the same field names, so a
/// canonical round trip is lossless and a shape that stopped mirroring stops
/// decoding.
pub(super) fn transcode_contract<S: Serialize + ?Sized, T: DeserializeOwned>(
    name: &'static str,
    source: &S,
) -> Result<T, ScenarioRuntimeError> {
    transcode(source).map_err(|error| projection(format!("cannot project a {name}: {error}")))
}

fn transcode<S: Serialize + ?Sized, T: DeserializeOwned>(
    source: &S,
) -> Result<T, ScenarioRuntimeError> {
    let value = serde_json::to_value(source).map_err(|error| projection(error.to_string()))?;
    serde_json::from_value(value).map_err(|error| projection(error.to_string()))
}

fn projection(detail: impl Into<String>) -> ScenarioRuntimeError {
    ScenarioRuntimeError::DocumentProjection {
        detail: detail.into(),
    }
}
