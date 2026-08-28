use pilotage_mission_core::{
    Digest as MissionDigest, EngineStart, ExecutionTarget, NavigationDataIdentity, WallDeadline,
};

use crate::{
    AdapterError, ArtifactIdentity, CampaignBackend, CampaignMissionRuntime, Digest, Journal,
    ScenarioRuntime, ScenarioStopReason, TuneError, mission_document_from_scenario,
};

use super::RunContext;

pub(super) fn admit_campaign_mission<B: CampaignBackend>(
    journal: &Journal,
    context: &RunContext<'_>,
    backend: &B,
) -> Result<CampaignMissionRuntime, TuneError> {
    journal.ensure_usable()?;
    attest_runtime(journal, backend)?;
    let scenario = backend
        .scenario_document_blocking(context.scenario)
        .map_err(|source| runtime_adapter_error(backend, "resolve scenario document", source))?;
    validate_scenario_artifact(context, &scenario)?;
    let timeout_ns = u64::from(context.scenario.sample_timeout_ms).saturating_mul(1_000_000);
    let document = mission_document_from_scenario(
        &scenario,
        navigation_identity(context.scenario.digest),
        0,
        timeout_ns,
    )
    .map_err(|source| mission_error(backend, "admit scenario document", source))?;
    CampaignMissionRuntime::attest_capabilities(&document, backend.scenario_runtime())
        .map_err(|source| mission_error(backend, "admit scenario capabilities", source))?;
    let duration_ns = timeout_ns
        .saturating_mul(u64::from(context.scenario.max_samples))
        .max(1);
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

fn validate_scenario_artifact(
    context: &RunContext<'_>,
    scenario: &pilotage_trial::Scenario,
) -> Result<(), TuneError> {
    if scenario.id != context.scenario.id {
        return Err(scenario_mismatch(
            "the resolved scenario id differs from its reference",
        ));
    }
    let digest = scenario.canonical_digest().map_err(|error| {
        scenario_mismatch(&format!("cannot digest the resolved scenario: {error}"))
    })?;
    if digest != context.scenario.digest {
        return Err(scenario_mismatch(
            "the resolved scenario content differs from its reference",
        ));
    }
    Ok(())
}

fn scenario_mismatch(detail: &str) -> TuneError {
    TuneError::ReceiptMismatch {
        operation: "resolve scenario document",
        detail: detail.to_owned(),
    }
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

fn navigation_identity(digest: Digest) -> NavigationDataIdentity {
    NavigationDataIdentity {
        cycle: "calibration".to_owned(),
        snapshot_id: "scenario-artifact".to_owned(),
        snapshot_digest: MissionDigest::from_bytes(*digest.as_bytes()),
    }
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
