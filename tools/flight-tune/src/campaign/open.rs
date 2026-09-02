use std::path::Path;

use crate::{
    CampaignBackend, Candidate, GateEvaluator, Journal, MetricEvaluator, ProposalStrategy,
    RunTerminalAdapter, RuntimeIdentities, SearchStage, SimulatorCapability,
    SimulatorVehicleAdapter, SimulatorVehicleFactory, TuneError, VehicleBinding,
};

use super::{Tuner, evaluate, reconcile, session};

mod transaction;

use transaction::OpenTransaction;

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
    /// One open transaction owns the simulator session and the vehicle
    /// binding until every open check passes. A check that fails first
    /// releases what the attempt acquired, in reverse acquisition order.
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
        backend: B,
        vehicle_factory: F,
        mut gates: G,
        mut metric: M,
        strategy: P,
        mut journal: Journal,
    ) -> Result<Self, TuneError>
    where
        F: SimulatorVehicleFactory<Adapter = V>,
    {
        journal.ensure_usable()?;
        let mut transaction = OpenTransaction::begin(backend, &vehicle_factory, &journal)?;
        transaction.reconcile_prior_open_blocking()?;
        let capability = transaction.open_session_blocking(&journal)?;
        if let Err(unusable) = journal.ensure_usable() {
            return Err(transaction.fail(unusable));
        }
        let mut vehicle =
            transaction.bind_vehicle_blocking(&journal, vehicle_factory, &capability)?;
        let remaining = complete_open_checks_blocking(
            &mut journal,
            &stage,
            transaction.backend_mut(),
            &mut vehicle,
            &capability,
            &mut gates,
            &mut metric,
        );
        if let Err(primary) = remaining {
            return Err(transaction.fail(primary));
        }
        Ok(Self {
            stage,
            backend: transaction.commit(),
            vehicle,
            capability,
            gates,
            metric,
            strategy,
            journal,
        })
    }
}

/// Runs every open check that needs both acquired resources.
///
/// These checks are the last ones before the commit. They read and write
/// the simulator and the vehicle, so a failure here still has to release
/// both, which is why they run inside the transaction rather than on a
/// tuner that already owns them.
fn complete_open_checks_blocking<B, V, G, M>(
    journal: &mut Journal,
    stage: &SearchStage,
    backend: &mut B,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
    gates: &mut G,
    metric: &mut M,
) -> Result<(), TuneError>
where
    B: CampaignBackend,
    V: SimulatorVehicleAdapter + RunTerminalAdapter,
    G: GateEvaluator,
    M: MetricEvaluator,
{
    evaluate::recover_pending_for_open_blocking(
        journal, stage, backend, vehicle, capability, gates, metric,
    )?;
    reconcile::settled_candidate_blocking(journal, vehicle, capability)
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
