use crate::journal::CampaignPhase;
use crate::{
    CampaignBackend, FinalQualificationOutcome, GateEvaluator, Journal, MetricEvaluator,
    ProposalStrategy, RunTerminalAdapter, SimulatorCapability, SimulatorVehicleAdapter, TuneError,
    VehicleBinding,
};

use super::{Tuner, evaluate, invalid_state};

/// Reactivates the candidate the journal states is settled.
///
/// The open transaction runs this before it commits and the tuner runs it
/// after every operation, so it takes the parts rather than a tuner: at
/// open there is no tuner to take.
///
/// # Errors
///
/// Returns [`TuneError`] when an attempt is still pending, when the
/// candidate cannot be read, or when the vehicle refuses it.
pub(super) fn settled_candidate_blocking<V>(
    journal: &Journal,
    vehicle: &mut VehicleBinding<V>,
    capability: &SimulatorCapability,
) -> Result<(), TuneError>
where
    V: SimulatorVehicleAdapter,
{
    journal.ensure_usable()?;
    if journal.state().pending.is_some() {
        return Err(invalid_state(
            "reconcile active candidate",
            "an attempt still requires cleanup",
        ));
    }
    let digest = settled_candidate_digest(journal);
    let candidate = journal.read_candidate(digest)?;
    evaluate::ensure_settled_candidate_blocking(journal, vehicle, capability, &candidate, digest)
}

fn settled_candidate_digest(journal: &Journal) -> crate::Digest {
    let initial = journal.session().initial_candidate_digest;
    match journal.phase() {
        CampaignPhase::Searching => journal.state().training_incumbent,
        CampaignPhase::Frozen => initial,
        CampaignPhase::PromotionClosed => journal.state().settlement_candidate(initial),
        CampaignPhase::Sealed
            if journal.state().final_outcome == Some(FinalQualificationOutcome::Qualified) =>
        {
            journal.state().settlement_candidate(initial)
        }
        CampaignPhase::Sealed => initial,
    }
}

impl<B, V, G, M, P> Tuner<B, V, G, M, P>
where
    B: CampaignBackend,
    V: SimulatorVehicleAdapter + RunTerminalAdapter,
    G: GateEvaluator,
    M: MetricEvaluator,
    P: ProposalStrategy,
{
    pub(super) fn reconcile_settled_candidate_blocking(&mut self) -> Result<(), TuneError> {
        settled_candidate_blocking(&self.journal, &mut self.vehicle, &self.capability)
    }

    pub(super) fn finish_with_candidate_reconciliation_blocking(
        &mut self,
        operation: &'static str,
        primary: Result<(), TuneError>,
    ) -> Result<(), TuneError> {
        if let Err(poisoned) = self.journal.ensure_usable() {
            return match primary {
                Ok(()) => Err(poisoned),
                Err(primary) => Err(primary),
            };
        }
        if self.journal.state().pending.is_some() {
            return primary.and_then(|()| {
                Err(invalid_state(
                    operation,
                    "a successful operation left an attempt pending",
                ))
            });
        }
        let reconciliation = self.reconcile_settled_candidate_blocking();
        match (primary, reconciliation) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(primary), Ok(())) => Err(primary),
            (Ok(()), Err(reconciliation)) => Err(reconciliation),
            (Err(primary), Err(reconciliation)) => {
                Err(TuneError::OperationAndReconciliationFailed {
                    operation,
                    primary: Box::new(primary),
                    reconciliation: Box::new(reconciliation),
                })
            }
        }
    }
}
