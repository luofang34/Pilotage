use crate::journal::CampaignPhase;
use crate::{
    FinalQualificationOutcome, GateEvaluator, MetricEvaluator, ProposalStrategy, SimulatorBackend,
    SimulatorVehicleAdapter, TuneError,
};

use super::{Tuner, evaluate, invalid_state};

impl<B, V, G, M, P> Tuner<B, V, G, M, P>
where
    B: SimulatorBackend,
    V: SimulatorVehicleAdapter,
    G: GateEvaluator,
    M: MetricEvaluator,
    P: ProposalStrategy,
{
    pub(super) fn reconcile_settled_candidate_blocking(&mut self) -> Result<(), TuneError> {
        self.journal.ensure_usable()?;
        if self.journal.state().pending.is_some() {
            return Err(invalid_state(
                "reconcile active candidate",
                "an attempt still requires cleanup",
            ));
        }
        let digest = self.settled_candidate_digest();
        let candidate = self.journal.read_candidate(digest)?;
        evaluate::ensure_settled_candidate_blocking(
            &self.journal,
            &mut self.vehicle,
            &self.capability,
            &candidate,
            digest,
        )
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

    fn settled_candidate_digest(&self) -> crate::Digest {
        let initial = self.journal.session().initial_candidate_digest;
        match self.journal.phase() {
            CampaignPhase::Searching => self.journal.state().training_incumbent,
            CampaignPhase::Frozen => initial,
            CampaignPhase::PromotionClosed => {
                self.journal.state().selected_release_candidate(initial)
            }
            CampaignPhase::Sealed
                if self.journal.state().final_outcome
                    == Some(FinalQualificationOutcome::Qualified) =>
            {
                self.journal.state().selected_release_candidate(initial)
            }
            CampaignPhase::Sealed => initial,
        }
    }
}
