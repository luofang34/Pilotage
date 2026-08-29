//! The direct authority and direct evidence of one run.
//!
//! The transport owns sending an exact step. What one run owns is the rest
//! of it: the durable ledger that brackets every send, the validation each
//! record has to pass before it can be scored, and the single digest that
//! binds the whole set to the run receipt the campaign seals.
//!
//! The authority exists only for a run whose mission actually commands the
//! direct family. A run that never asks for a direct step never mints one,
//! so the path is absent rather than merely unused.

use flight_tune::{ArtifactIdentity, Digest};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::direct_transport::{
    DirectBaselineRequest, DirectCommandRecord, DirectCommandSender, DirectStepRequest,
    DirectTransport, DirectTransportRequest,
};

use super::AviateRuntimeError;
use super::phase::direct::DirectStepOutcome;
use super::phase::direct::ledger::{DirectIntentLedger, DirectIntentStore, DirectRecoveryOutcome};
use super::phase::direct::readback::PublicationContext;

/// The supported direct run-evidence document schema.
pub const DIRECT_RUN_EVIDENCE_SCHEMA_VERSION: u16 = 1;

const DIRECT_EVIDENCE_DOMAIN: &[u8] = b"pilotage-aviate-direct-run-evidence-v1\0";

/// The complete direct-command evidence of one run.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DirectRunEvidence {
    /// Document schema version.
    pub schema_version: u16,
    /// The exact run intent this evidence belongs to.
    pub run_intent_digest: Digest,
    /// The direct transport that sent every command.
    pub transport_identity_digest: Digest,
    /// The runtime implementation that drove the transport.
    pub runtime_identity: ArtifactIdentity,
    /// Every validated direct command, in the order the run sent them.
    pub records: Vec<DirectCommandRecord>,
}

impl DirectRunEvidence {
    /// The identity that a run receipt binds this evidence by.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when the document cannot be encoded.
    pub fn digest(&self) -> Result<Digest, AviateRuntimeError> {
        let bytes = serde_json::to_vec(self).map_err(|source| AviateRuntimeError::Encode {
            document: "direct run evidence",
            source,
        })?;
        let mut hasher = Sha256::new();
        hasher.update(DIRECT_EVIDENCE_DOMAIN);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
        Ok(Digest::from_bytes(hasher.finalize().into()))
    }

    /// Rejects evidence that does not belong to one exact run intent.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when a record names another run
    /// intent, another transport, or the document names another runtime.
    pub fn require_bound(
        &self,
        run_intent_digest: Digest,
        runtime_identity: &ArtifactIdentity,
    ) -> Result<(), AviateRuntimeError> {
        if self.schema_version != DIRECT_RUN_EVIDENCE_SCHEMA_VERSION
            || self.run_intent_digest != run_intent_digest
            || &self.runtime_identity != runtime_identity
        {
            return Err(AviateRuntimeError::DirectEvidenceUnbound);
        }
        if self.records.iter().any(|record| {
            record.run_intent_digest != run_intent_digest
                || record.transport_identity_digest != self.transport_identity_digest
        }) {
            return Err(AviateRuntimeError::DirectEvidenceUnbound);
        }
        Ok(())
    }
}

/// The direct authority that one run holds.
#[derive(Debug)]
pub struct DirectRunAuthority {
    transport: DirectTransport,
    ledger: DirectIntentLedger,
    context: PublicationContext,
    records: Vec<DirectCommandRecord>,
}

impl DirectRunAuthority {
    /// Authorizes the simulator-only direct path for one run.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when the execution target is a real
    /// vehicle, when a binding does not accept the tuning session, or when
    /// the run intent is missing.
    pub fn authorize<S: DirectCommandSender + ?Sized>(
        request: &DirectTransportRequest<'_>,
        sender: &S,
        run_intent_digest: Digest,
    ) -> Result<Self, AviateRuntimeError> {
        if run_intent_digest.is_zero() {
            return Err(AviateRuntimeError::IncompleteIdentity {
                detail: "the direct authority has no run intent".to_owned(),
            });
        }
        let transport = DirectTransport::authorize(request, sender)
            .map_err(|source| AviateRuntimeError::DirectTransport { source })?;
        let context = PublicationContext {
            run_intent_digest,
            transport_identity_digest: transport.session().identity_digest(),
            tolerance: transport.tolerance(),
        };
        Ok(Self {
            transport,
            ledger: DirectIntentLedger::new(),
            context,
            records: Vec::new(),
        })
    }

    /// The exact direct transport this run commands through.
    pub const fn transport_mut(&mut self) -> &mut DirectTransport {
        &mut self.transport
    }

    /// The exact direct transport this run commands through.
    #[must_use]
    pub const fn transport(&self) -> &DirectTransport {
        &self.transport
    }

    /// The durable ledger that brackets every direct send.
    pub const fn ledger_mut(&mut self) -> &mut DirectIntentLedger {
        &mut self.ledger
    }

    /// What every published record has to agree with.
    #[must_use]
    pub const fn publication_context(&self) -> &PublicationContext {
        &self.context
    }

    /// Adds one validated direct record to this run's evidence.
    pub fn record(&mut self, record: DirectCommandRecord) {
        self.records.push(record);
    }

    /// Rejects a resume whose durable ledger cannot say what was sent.
    ///
    /// A prepared intent with no durable result means the command may or
    /// may not have reached the flight controller. The run is not scorable
    /// either way, so the ambiguity ends the run instead of resuming it.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when the ledger is unreadable, or
    /// when a prepared direct command has no durable result.
    pub fn require_resolved_ledger<S: DirectIntentStore + ?Sized>(
        &self,
        store: &S,
    ) -> Result<DirectRecoveryOutcome, AviateRuntimeError> {
        let outcome = store.read_state(self.ledger.sequence())?;
        if let DirectRecoveryOutcome::Ambiguous(intent) = &outcome {
            return Err(AviateRuntimeError::AmbiguousDirectCommand {
                sequence: intent.sequence,
            });
        }
        Ok(outcome)
    }

    /// Seals every validated direct record as this run's direct evidence.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when a prepared command is still open
    /// or a record does not bind this run.
    pub fn seal(
        &self,
        runtime_identity: &ArtifactIdentity,
    ) -> Result<DirectRunEvidence, AviateRuntimeError> {
        if self.ledger.is_open() {
            return Err(AviateRuntimeError::AmbiguousDirectCommand {
                sequence: self.ledger.sequence(),
            });
        }
        let evidence = DirectRunEvidence {
            schema_version: DIRECT_RUN_EVIDENCE_SCHEMA_VERSION,
            run_intent_digest: self.context.run_intent_digest,
            transport_identity_digest: self.context.transport_identity_digest,
            runtime_identity: runtime_identity.clone(),
            records: self.records.clone(),
        };
        evidence.require_bound(self.context.run_intent_digest, runtime_identity)?;
        Ok(evidence)
    }

    /// Removes every direct authority this run holds.
    pub fn revoke(&mut self) {
        let _receipt = self.transport.revoke();
    }
}

/// The vehicle state one direct stimulus is entered from.
///
/// The attitude comes from the frame that opens the stimulus, so the
/// frozen baseline is the state the vehicle was actually in when the
/// scored window began.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectEntryState {
    /// The measured roll at stimulus entry, in radians.
    pub roll_rad: f64,
    /// The measured pitch at stimulus entry, in radians.
    pub pitch_rad: f64,
    /// The measured heading at stimulus entry, in radians.
    pub yaw_rad: f64,
}

/// The configured shape of one run's direct baseline block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectBaselinePolicy {
    /// The identified hover trim that a vertical stimulus measures from.
    pub hover_trim: f64,
    /// The largest number of commands in the baseline block.
    pub max_commands: u32,
}

/// The exact direct control that one runtime commands through.
///
/// A runtime built with [`NoDirectControl`] has no direct path at all: the
/// sender, the transport, and the durable ledger are absent from the type,
/// so a mission that asks for the direct family is refused rather than
/// quietly shaped through the operator law.
pub trait DirectControl {
    /// Freezes the direct baseline every step of this run is measured from.
    ///
    /// The call is idempotent: a run freezes one baseline, at the frame
    /// that opens its first direct stimulus, and every later step measures
    /// from that same frozen state.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when this runtime has no direct
    /// authority or the baseline block does not settle.
    fn ensure_baseline_blocking(
        &mut self,
        entry: DirectEntryState,
    ) -> Result<(), AviateRuntimeError>;

    /// Sends one exact direct command, bracketed by the durable ledger.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when this runtime has no direct
    /// authority, when the ledger cannot record the command, or when the
    /// resulting record fails publication validation.
    fn command_blocking(
        &mut self,
        stimulus: &DirectStepRequest,
        release: bool,
    ) -> Result<DirectStepOutcome, AviateRuntimeError>;

    /// Seals the direct evidence of this run, when the run has any.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when a prepared command has no
    /// durable result or a record does not bind this run.
    fn seal(
        &self,
        runtime_identity: &ArtifactIdentity,
    ) -> Result<Option<DirectRunEvidence>, AviateRuntimeError>;

    /// Removes every direct authority this run holds.
    fn revoke(&mut self);
}

/// A runtime with no direct path.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NoDirectControl;

impl DirectControl for NoDirectControl {
    fn ensure_baseline_blocking(
        &mut self,
        _entry: DirectEntryState,
    ) -> Result<(), AviateRuntimeError> {
        Err(AviateRuntimeError::NoDirectAuthority)
    }

    fn command_blocking(
        &mut self,
        _stimulus: &DirectStepRequest,
        _release: bool,
    ) -> Result<DirectStepOutcome, AviateRuntimeError> {
        Err(AviateRuntimeError::NoDirectAuthority)
    }

    fn seal(
        &self,
        _runtime_identity: &ArtifactIdentity,
    ) -> Result<Option<DirectRunEvidence>, AviateRuntimeError> {
        Ok(None)
    }

    fn revoke(&mut self) {}
}

/// The simulator-only direct path of one run.
///
/// It owns the authority, the exact command sender, and the durable
/// ledger together, because none of the three is safe without the other
/// two: authority without a ledger cannot say what it sent, and a ledger
/// without authority has nothing to record.
#[derive(Debug)]
pub struct SimulatorDirectControl<S, L> {
    authority: DirectRunAuthority,
    sender: S,
    store: L,
    baseline: DirectBaselinePolicy,
}

impl<S: DirectCommandSender, L: DirectIntentStore> SimulatorDirectControl<S, L> {
    /// Authorizes the simulator-only direct path for one run.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when the execution target is a real
    /// vehicle, when a binding does not accept the tuning session, or when
    /// the durable ledger already holds an unresolved prepared command.
    pub fn authorize(
        request: &DirectTransportRequest<'_>,
        sender: S,
        store: L,
        run_intent_digest: Digest,
        baseline: DirectBaselinePolicy,
    ) -> Result<Self, AviateRuntimeError> {
        if !baseline.hover_trim.is_finite() || baseline.max_commands == 0 {
            return Err(AviateRuntimeError::InvalidValue {
                field: "direct baseline policy",
            });
        }
        let authority = DirectRunAuthority::authorize(request, &sender, run_intent_digest)?;
        authority.require_resolved_ledger(&store)?;
        Ok(Self {
            authority,
            sender,
            store,
            baseline,
        })
    }

    /// The direct authority this run holds.
    #[must_use]
    pub const fn authority(&self) -> &DirectRunAuthority {
        &self.authority
    }
}

impl<S: DirectCommandSender, L: DirectIntentStore> DirectControl for SimulatorDirectControl<S, L> {
    fn ensure_baseline_blocking(
        &mut self,
        entry: DirectEntryState,
    ) -> Result<(), AviateRuntimeError> {
        if self.authority.transport().baseline().is_some() {
            return Ok(());
        }
        let request = DirectBaselineRequest {
            measured_roll_rad: entry.roll_rad,
            measured_pitch_rad: entry.pitch_rad,
            measured_yaw_rad: entry.yaw_rad,
            hover_trim: self.baseline.hover_trim,
            run_intent_digest: self.authority.publication_context().run_intent_digest,
            max_commands: self.baseline.max_commands,
        };
        super::phase::direct::freeze_baseline_blocking(
            &mut self.authority,
            &mut self.sender,
            &request,
        )
    }

    fn command_blocking(
        &mut self,
        stimulus: &DirectStepRequest,
        release: bool,
    ) -> Result<DirectStepOutcome, AviateRuntimeError> {
        super::phase::direct::send_step_blocking(
            &mut self.authority,
            &mut self.sender,
            &mut self.store,
            stimulus,
            release,
        )
    }

    fn seal(
        &self,
        runtime_identity: &ArtifactIdentity,
    ) -> Result<Option<DirectRunEvidence>, AviateRuntimeError> {
        self.authority.seal(runtime_identity).map(Some)
    }

    fn revoke(&mut self) {
        self.authority.revoke();
    }
}
