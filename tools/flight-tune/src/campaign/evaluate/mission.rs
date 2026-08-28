use pilotage_mission_core::{EngineStart, ExecutionTarget, WallDeadline};

use crate::{
    AdapterError, ArtifactIdentity, CampaignBackend, CampaignMissionRuntime, Journal,
    ScenarioRuntime, ScenarioStopReason, TuneError,
};

use super::RunContext;

pub(super) fn admit_campaign_mission<B: CampaignBackend>(
    journal: &Journal,
    context: &RunContext<'_>,
    backend: &B,
) -> Result<CampaignMissionRuntime, TuneError> {
    journal.ensure_usable()?;
    attest_runtime(journal, backend)?;
    let document = backend
        .mission_document_blocking(context.scenario)
        .map_err(|source| runtime_adapter_error(backend, "resolve mission document", source))?;
    context.scenario.verify_document(&document)?;
    CampaignMissionRuntime::attest_capabilities(&document, backend.scenario_runtime())
        .map_err(|source| mission_error(backend, "admit mission capabilities", source))?;
    let duration_ns = context.scenario.run_duration_ns();
    CampaignMissionRuntime::admit(document.clone(), engine_start(&document, duration_ns))
        .map_err(|source| mission_error(backend, "start mission engine", source))
}

pub(super) fn start_campaign_action_port<B: CampaignBackend>(
    journal: &Journal,
    context: &RunContext<'_>,
    backend: &mut B,
    mission: &mut CampaignMissionRuntime,
) -> Result<(), TuneError> {
    journal.ensure_usable()?;
    let expected = expected_scenario_runtime(journal)?.clone();
    mission
        .prepare_action_port_blocking(
            &expected,
            backend.scenario_runtime_mut(),
            &context.execution,
        )
        .map_err(|source| mission_error(backend, "prepare scenario action port", source))?;
    if let Err(primary) = journal.ensure_usable() {
        return contain_prepared_runtime(backend, mission, primary);
    }
    mission
        .start_prepared_action_port_blocking(backend.scenario_runtime_mut())
        .map_err(|source| mission_error(backend, "start scenario action port", source))
}

fn contain_prepared_runtime<B: CampaignBackend>(
    backend: &mut B,
    mission: &mut CampaignMissionRuntime,
    primary: TuneError,
) -> Result<(), TuneError> {
    let adapter = backend.scenario_runtime().identity().id.clone();
    let containment = mission.stop_and_cleanup_blocking(
        backend.scenario_runtime_mut(),
        ScenarioStopReason::ExecutionError,
        mission.last_consumed_source_sequence(),
    );
    match containment {
        Ok(()) => Err(primary),
        Err(source) => Err(TuneError::OperationAndTerminalFailed {
            operation: "authorize scenario action-port start",
            primary: Box::new(primary),
            terminal: Box::new(TuneError::Adapter {
                adapter,
                operation: "contain prepared scenario action port",
                source: AdapterError::new(source.to_string()),
            }),
        }),
    }
}

fn attest_runtime<B: CampaignBackend>(journal: &Journal, backend: &B) -> Result<(), TuneError> {
    backend
        .attest_scenario_runtime_blocking()
        .map_err(|source| runtime_adapter_error(backend, "attest scenario runtime", source))?;
    let expected = expected_scenario_runtime(journal)?;
    CampaignMissionRuntime::attest_action_port(expected, backend.scenario_runtime())
        .map_err(|source| mission_error(backend, "attest scenario action port", source))
}

fn engine_start(
    document: &pilotage_mission_core::MissionDocument,
    duration_ns: u64,
) -> EngineStart {
    EngineStart {
        host_target: ExecutionTarget::Simulator,
        simulator_time_ns: 0,
        wall_time_ns: 0,
        wall_deadline: WallDeadline {
            mission_content_digest: document.identity.content_digest,
            expires_at_ns: duration_ns,
        },
    }
}

fn expected_scenario_runtime(journal: &Journal) -> Result<&ArtifactIdentity, TuneError> {
    journal
        .session()
        .runtimes
        .scenario_runtime
        .as_ref()
        .ok_or_else(|| TuneError::InvalidIdentity {
            detail: "the scenario runtime uses the prior identity domain".to_owned(),
        })
}

fn mission_error<B: CampaignBackend>(
    backend: &B,
    operation: &'static str,
    source: crate::ScenarioRuntimeError,
) -> TuneError {
    runtime_adapter_error(backend, operation, AdapterError::new(source.to_string()))
}

fn runtime_adapter_error<B: CampaignBackend>(
    backend: &B,
    operation: &'static str,
    source: AdapterError,
) -> TuneError {
    TuneError::Adapter {
        adapter: backend.scenario_runtime().identity().id.clone(),
        operation,
        source,
    }
}
