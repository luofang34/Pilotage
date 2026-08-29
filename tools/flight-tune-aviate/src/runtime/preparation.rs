//! Admitting one run, and holding every receipt to its exact intent.
//!
//! A run has one identity: the digest of its [`RunExecutionContext`].
//! Preparation, scenario start, candidate activation, and controller
//! readback each return a receipt, and every one of them has to carry that
//! same digest. This module is where the comparison happens, and it
//! happens before the next external action rather than after it, so a
//! receipt for another run stops the sequence instead of being recorded
//! next to work it does not describe.

use flight_tune::{
    ArtifactIdentity, CandidateReceipt, Digest, ExecutionTarget, MissionDocument,
    RunExecutionContext, RunPreparationReceipt, ScenarioStartReceipt,
};

use super::AviateRuntimeError;
use super::identity::AviateRuntimeIdentity;

/// One admitted run and the identity every receipt has to name.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedRun {
    context: RunExecutionContext,
    run_intent_digest: Digest,
    mission_content_digest: Digest,
    runtime_identity: ArtifactIdentity,
}

impl PreparedRun {
    /// Admits one mission for one exact run identity.
    ///
    /// The runtime attests before the run is admitted, so a runtime whose
    /// production inputs no longer describe it cannot prepare anything.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when the runtime no longer attests,
    /// when the document is not a simulator mission, or when the document
    /// content differs from the content the run intent names.
    pub fn admit(
        document: &MissionDocument,
        context: &RunExecutionContext,
        runtime: &AviateRuntimeIdentity,
    ) -> Result<Self, AviateRuntimeError> {
        runtime.attest()?;
        document
            .validate_for_target(ExecutionTarget::Simulator)
            .map_err(|source| AviateRuntimeError::MissionRejected {
                detail: source.to_string(),
            })?;
        let mission_content_digest = document.calculate_content_digest().map_err(|source| {
            AviateRuntimeError::MissionRejected {
                detail: source.to_string(),
            }
        })?;
        let run_intent_digest = context
            .digest()
            .map_err(|source| AviateRuntimeError::InvalidIdentity { source })?;
        // Both crates carry their own digest type over the same 32 bytes,
        // so the comparison crosses by bytes.
        if mission_content_digest.as_bytes() != context.mission_content_digest().as_bytes() {
            return Err(AviateRuntimeError::MissionRejected {
                detail: "the mission document is not the one the run intent names".to_owned(),
            });
        }
        Ok(Self {
            context: context.clone(),
            run_intent_digest,
            mission_content_digest: Digest::from_bytes(*mission_content_digest.as_bytes()),
            runtime_identity: runtime.identity().clone(),
        })
    }

    /// The exact run intent identity.
    #[must_use]
    pub const fn run_intent_digest(&self) -> Digest {
        self.run_intent_digest
    }

    /// The exact mission content this run flies.
    #[must_use]
    pub const fn mission_content_digest(&self) -> Digest {
        self.mission_content_digest
    }

    /// The runtime implementation that admitted this run.
    #[must_use]
    pub const fn runtime_identity(&self) -> &ArtifactIdentity {
        &self.runtime_identity
    }

    /// The complete run identity.
    #[must_use]
    pub const fn context(&self) -> &RunExecutionContext {
        &self.context
    }

    /// The tuning session that owns this run.
    #[must_use]
    pub const fn session_digest(&self) -> Digest {
        self.context.tuning_session_digest()
    }

    /// Rejects a preparation receipt for another run or session.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when a digest differs.
    pub fn require_preparation(
        &self,
        receipt: &RunPreparationReceipt,
    ) -> Result<(), AviateRuntimeError> {
        self.require_match(
            "preparation",
            receipt.session_digest,
            Some(receipt.run_intent_digest),
        )
    }

    /// Rejects a scenario start receipt for another run, session, or mission.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when a digest differs.
    pub fn require_start(&self, receipt: &ScenarioStartReceipt) -> Result<(), AviateRuntimeError> {
        self.require_match(
            "scenario start",
            receipt.session_digest,
            Some(receipt.run_intent_digest),
        )?;
        if receipt.applied_mission_content_digest != self.mission_content_digest {
            return Err(AviateRuntimeError::ReceiptMismatch {
                receipt: "scenario start",
            });
        }
        if receipt.seed != self.context.seed() {
            return Err(AviateRuntimeError::ReceiptMismatch {
                receipt: "scenario start",
            });
        }
        Ok(())
    }

    /// Rejects a candidate apply and readback receipt for another run.
    ///
    /// The apply digest and the readback digest have to agree with the
    /// requested candidate as well, so a controller that accepted one value
    /// and reports another stops the run here.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when a digest differs.
    pub fn require_candidate(&self, receipt: &CandidateReceipt) -> Result<(), AviateRuntimeError> {
        self.require_match(
            "candidate",
            receipt.session_digest,
            receipt.run_intent_digest,
        )?;
        if receipt.requested_digest != self.context.candidate_digest() {
            return Err(AviateRuntimeError::ReceiptMismatch {
                receipt: "candidate request",
            });
        }
        if receipt.applied_digest != receipt.requested_digest {
            return Err(AviateRuntimeError::ReceiptMismatch {
                receipt: "candidate apply",
            });
        }
        if receipt.readback_digest != receipt.requested_digest {
            return Err(AviateRuntimeError::ReceiptMismatch {
                receipt: "candidate readback",
            });
        }
        Ok(())
    }

    /// Rejects a runtime whose identity is not the admitted one.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when the runtime identity differs.
    pub fn require_runtime(
        &self,
        runtime: &AviateRuntimeIdentity,
    ) -> Result<(), AviateRuntimeError> {
        runtime.require_frozen(&self.runtime_identity)
    }

    fn require_match(
        &self,
        receipt: &'static str,
        session_digest: Digest,
        run_intent_digest: Option<Digest>,
    ) -> Result<(), AviateRuntimeError> {
        if session_digest != self.session_digest() {
            return Err(AviateRuntimeError::ReceiptMismatch { receipt });
        }
        match run_intent_digest {
            Some(digest) if digest == self.run_intent_digest => Ok(()),
            Some(_) | None => Err(AviateRuntimeError::ReceiptMismatch { receipt }),
        }
    }
}
