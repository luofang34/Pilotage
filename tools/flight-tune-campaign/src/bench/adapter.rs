//! The vehicle half of the bench: settling a law and holding its receipts.

use flight_tune::{
    AdapterError, Candidate, CandidateReceipt, CandidateTransitionReceipt,
    CandidateTransitionRequest, Digest, RunExecutionContext, SimulatorCapability,
    SimulatorVehicleAdapter,
};

use super::{BenchHandle, response_from, to_adapter};

/// The vehicle half of the contract: it settles a candidate's command law.
#[derive(Debug)]
pub struct BenchVehicleAdapter {
    handle: BenchHandle,
}

impl BenchVehicleAdapter {
    /// Creates one adapter over shared state.
    #[must_use]
    pub fn new(handle: BenchHandle) -> Self {
        Self { handle }
    }
}

impl SimulatorVehicleAdapter for BenchVehicleAdapter {
    fn authorize_candidate_transition(
        &self,
        request: &CandidateTransitionRequest,
    ) -> Result<CandidateTransitionReceipt, AdapterError> {
        CandidateTransitionReceipt::authorized(request)
            .map_err(|error| AdapterError::new(error.to_string()))
    }

    fn ensure_settled_candidate_blocking(
        &mut self,
        _capability: &SimulatorCapability,
        candidate: &Candidate,
        candidate_digest: Digest,
    ) -> Result<CandidateReceipt, AdapterError> {
        let mut settled = self.handle.0.borrow_mut();
        // Repeating the request must not rewrite the law, so a restart can be
        // reconciled without disturbing a vehicle already on the candidate.
        if settled.digest != Some(candidate_digest) {
            settled.response = Some(response_from(candidate)?);
            settled.digest = Some(candidate_digest);
        }
        Ok(CandidateReceipt {
            session_digest: _capability.session_digest(),
            requested_digest: candidate_digest,
            applied_digest: candidate_digest,
            // The bench applies the law in process, so what it reads back is
            // what it applied. A vehicle over a link reads its controller.
            readback_digest: candidate_digest,
            // Idle reconciliation settles a candidate without a run behind it.
            run_intent_digest: None,
        })
    }

    fn ensure_candidate_for_run_blocking(
        &mut self,
        capability: &SimulatorCapability,
        context: &RunExecutionContext,
        candidate: &Candidate,
        candidate_digest: Digest,
    ) -> Result<CandidateReceipt, AdapterError> {
        let mut receipt =
            self.ensure_settled_candidate_blocking(capability, candidate, candidate_digest)?;
        // The receipt names the run it was settled for, so a law applied for
        // one run cannot be read as evidence for another.
        receipt.run_intent_digest = Some(context.digest().map_err(to_adapter)?);
        Ok(receipt)
    }
}

impl flight_tune::RunTerminalAdapter for BenchVehicleAdapter {
    fn terminal_capabilities(&self) -> flight_tune::RunTerminalCapabilities {
        // The bench holds no external control path, no trace collector and no
        // supervised child: the law, the vehicle and the trace are all in this
        // process, and a run ends when the loop stops stepping. Advertising a
        // capability it does not have would have the engine wait for a stop
        // acknowledgement nothing will send.
        flight_tune::RunTerminalCapabilities::new(false, false, false)
    }

    fn bind_terminal_plan_blocking(
        &mut self,
        _capability: &SimulatorCapability,
        _binding: &flight_tune::RunBindingReceipt,
        _plan: &flight_tune::RunTerminalPlan,
    ) -> Result<(), AdapterError> {
        // Nothing to bind: with no external component to stop, the plan is
        // empty and accepting it is the whole of the work. Repeating the call
        // must be safe, and doing nothing twice is.
        Ok(())
    }

    fn causal_evidence_digest_blocking(
        &mut self,
        _capability: &SimulatorCapability,
        _binding: &flight_tune::RunBindingReceipt,
    ) -> Result<Digest, AdapterError> {
        // The causal evidence for an in-process run is the run itself: there
        // is no external trace to correlate against, so the identity stated
        // here is the one fixed value that says which trace this was.
        Ok(Digest::from_bytes([0x0c; 32]))
    }

    fn seal_terminal_receipt_blocking(
        &mut self,
        _capability: &SimulatorCapability,
        _binding: &flight_tune::RunBindingReceipt,
        receipt: &flight_tune::RunTerminalReceipt,
    ) -> Result<(), AdapterError> {
        // A seal the adapter acknowledges and does not keep is a seal the
        // engine cannot read back, and the engine is right to refuse one.
        // Sealing the same receipt twice keeps one copy, so a repeated call
        // after an uncertain acknowledgement is safe.
        let mut settled = self.handle.0.borrow_mut();
        if !settled.sealed.contains(receipt) {
            settled.sealed.push(receipt.clone());
        }
        Ok(())
    }

    fn recover_terminal_receipts_blocking(
        &mut self,
        _capability: &SimulatorCapability,
        _binding: &flight_tune::RunBindingReceipt,
    ) -> Result<Vec<flight_tune::RunTerminalReceipt>, AdapterError> {
        Ok(self.handle.0.borrow().sealed.clone())
    }
}
