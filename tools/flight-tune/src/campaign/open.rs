use std::path::Path;

use crate::{
    CampaignBackend, Candidate, GateEvaluator, Journal, MetricEvaluator, ProposalStrategy,
    RunTerminalAdapter, RuntimeIdentities, SearchStage, SessionChallenge, SimulatorCapability,
    SimulatorVehicleAdapter, SimulatorVehicleFactory, TuneError,
};

use super::{Tuner, evaluate, session};

impl<B, V, G, M, P> Tuner<B, V, G, M, P>
where
    B: CampaignBackend,
    V: SimulatorVehicleAdapter + RunTerminalAdapter,
    G: GateEvaluator,
    M: MetricEvaluator,
    P: ProposalStrategy,
{
    /// Opens a matching campaign or creates a new campaign.
    ///
    /// The constructor binds the vehicle adapter to the validated simulator
    /// session. It also cleans and quarantines an incomplete prepared attempt.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when identity, binding, recovery, or storage fails.
    #[allow(clippy::too_many_arguments)]
    pub fn open_or_resume<F>(
        journal_root: impl AsRef<Path>,
        stage: SearchStage,
        fixed_seed: u64,
        initial_candidate: Candidate,
        backend: B,
        vehicle_factory: F,
        gates: G,
        metric: M,
        strategy: P,
    ) -> Result<Self, TuneError>
    where
        F: SimulatorVehicleFactory<Adapter = V>,
    {
        let runtimes =
            validate_open_components(&backend, &vehicle_factory, &gates, &metric, &strategy)?;
        let journal = Journal::open_or_create(
            journal_root,
            &stage,
            fixed_seed,
            runtimes,
            &initial_candidate,
        )?;
        Self::finish_open(
            stage,
            backend,
            vehicle_factory,
            gates,
            metric,
            strategy,
            journal,
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn open_or_resume_with_faults<F>(
        journal_root: impl AsRef<Path>,
        stage: SearchStage,
        fixed_seed: u64,
        initial_candidate: Candidate,
        backend: B,
        vehicle_factory: F,
        gates: G,
        metric: M,
        strategy: P,
        faults: pilotage_durable_storage::FaultController,
    ) -> Result<Self, TuneError>
    where
        F: SimulatorVehicleFactory<Adapter = V>,
    {
        let runtimes =
            validate_open_components(&backend, &vehicle_factory, &gates, &metric, &strategy)?;
        let journal = Journal::open_or_create_with_faults(
            journal_root,
            &stage,
            fixed_seed,
            runtimes,
            &initial_candidate,
            faults,
        )?;
        Self::finish_open(
            stage,
            backend,
            vehicle_factory,
            gates,
            metric,
            strategy,
            journal,
        )
    }

    fn finish_open<F>(
        stage: SearchStage,
        mut backend: B,
        vehicle_factory: F,
        mut gates: G,
        mut metric: M,
        strategy: P,
        mut journal: Journal,
    ) -> Result<Self, TuneError>
    where
        F: SimulatorVehicleFactory<Adapter = V>,
    {
        let vehicle_identity = vehicle_factory.vehicle_identity().clone();
        journal.ensure_usable()?;
        let session_digest = journal.session_digest()?;
        let challenge = SessionChallenge::new(session_digest);
        let receipt = backend
            .open_session_blocking(&challenge)
            .map_err(|source| TuneError::Adapter {
                adapter: backend.simulator_identity().id.clone(),
                operation: "open simulator session",
                source,
            })?;
        session::validate_simulator_receipt(&journal, receipt)?;
        let capability = SimulatorCapability::new(receipt);
        journal.ensure_usable()?;
        let mut vehicle = vehicle_factory
            .bind_blocking(&capability)
            .map_err(|source| TuneError::Adapter {
                adapter: vehicle_identity.id,
                operation: "bind simulator vehicle",
                source,
            })?;
        session::validate_vehicle_binding(&journal, &vehicle)?;
        evaluate::recover_pending_for_open_blocking(
            &mut journal,
            &stage,
            &mut backend,
            &mut vehicle,
            &capability,
            &mut gates,
            &mut metric,
        )?;
        let mut tuner = Self {
            stage,
            backend,
            vehicle,
            capability,
            gates,
            metric,
            strategy,
            journal,
        };
        tuner.reconcile_settled_candidate_blocking()?;
        Ok(tuner)
    }
}

pub(crate) fn validate_open_components<B, F, G, M, P>(
    backend: &B,
    vehicle_factory: &F,
    gates: &G,
    metric: &M,
    strategy: &P,
) -> Result<RuntimeIdentities, TuneError>
where
    B: CampaignBackend,
    F: SimulatorVehicleFactory,
    G: GateEvaluator,
    M: MetricEvaluator,
    P: ProposalStrategy,
{
    session::validate_component_identities(backend, vehicle_factory, gates, metric, strategy)?;
    require_contact_state(backend.scenario_runtime())?;
    session::runtime_identities(backend, vehicle_factory, gates, metric, strategy)
}

/// Refuses a runtime that cannot report ground contact and crash state.
///
/// The crash gate is the floor of every campaign, and it reads a contact
/// signal. A backend that never declares one would give every sample an
/// absent value, which the gate refuses one execution at a time; refusing it
/// once at open states the same thing before a simulator is touched.
fn require_contact_state<R: crate::ScenarioRuntime>(runtime: &R) -> Result<(), TuneError> {
    if runtime
        .capabilities()
        .contains(&crate::MissionCapability::ContactState)
    {
        return Ok(());
    }
    Err(TuneError::InvalidStage {
        detail: format!(
            "the scenario runtime declares no contact state, which {} needs",
            crate::MANDATORY_CRASH_GATE_ID
        ),
    })
}
