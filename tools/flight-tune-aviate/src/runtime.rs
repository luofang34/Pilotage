//! The production Aviate scenario runtime.
//!
//! Every input that shapes an Aviate production run enters one identity.
//! The build script inventories the production sources under this module,
//! [`identity`] binds that inventory to the vehicle, transition validator,
//! adjacency policy, direct transport, and configuration, and the sealed
//! value becomes the driver's action-port identity. The simulator-neutral
//! harness composes it with the shared engine, so a changed production
//! input changes the scenario runtime identity that every receipt, journal
//! record, and campaign verification is bound by.
//!
//! The runtime attests before every external action. A runtime whose
//! sealed identity no longer describes its own document cannot reach a
//! journal, a process, a socket, the simulator, or the vehicle.
//!
//! SIM / NOT FOR FLIGHT.

pub mod conditions;
pub mod direct;
pub mod identity;
pub mod math;
pub mod phase;
pub mod preparation;
pub mod quality;
pub mod telemetry;
pub mod terminal;
pub mod timing;

use flight_tune::{
    ArtifactIdentity, MissionCapability, MissionDocument, ReceiptResult, RunExecutionContext,
    ScenarioFrame, ScenarioStopContext, TuneError,
};
use thiserror::Error;

use crate::action_port::{AviateActionDriver, AviateActionPortError, AviateVehicleDirective};
use crate::direct_transport::DirectTransportError;

use conditions::ConditionLedger;
use direct::DirectControl;
use identity::AviateRuntimeIdentity;
use phase::PhaseMachine;
use phase::transition::StartStateTolerance;
use preparation::PreparedRun;
use telemetry::VehicleSignals;
use terminal::{AviateRunSeal, RunClosure};
use timing::{FrameStamp, SampleClock};

/// One production Aviate runtime operation failed.
#[derive(Debug, Error)]
pub enum AviateRuntimeError {
    /// A bound runtime identity is not valid.
    #[error("the Aviate runtime identity is not valid: {source}")]
    InvalidIdentity {
        /// The identity validation failure.
        #[source]
        source: TuneError,
    },
    /// A runtime identity input is missing or zero.
    #[error("the Aviate runtime identity is incomplete: {detail}")]
    IncompleteIdentity {
        /// The stable identity detail.
        detail: String,
    },
    /// The runtime no longer matches the identity it was sealed with.
    #[error("the Aviate runtime identity changed")]
    RuntimeIdentityChanged,
    /// A document cannot be encoded.
    #[error("cannot encode the Aviate {document} document")]
    Encode {
        /// The document class.
        document: &'static str,
        /// The encoding failure.
        #[source]
        source: serde_json::Error,
    },
    /// A document cannot be decoded.
    #[error("cannot decode the Aviate {document} document")]
    Decode {
        /// The document class.
        document: &'static str,
        /// The decoding failure.
        #[source]
        source: serde_json::Error,
    },
    /// A numeric value is not usable.
    #[error("the Aviate runtime {field} is not a usable number")]
    InvalidValue {
        /// The invalid field.
        field: &'static str,
    },
    /// A frame arrived out of order.
    #[error("the Aviate runtime refused a frame: {detail}")]
    FrameOrder {
        /// The stable ordering detail.
        detail: &'static str,
    },
    /// A frame omits a state the vehicle port needs.
    #[error("the Aviate scenario frame omits the {field}")]
    IncompleteFrame {
        /// The absent field.
        field: &'static str,
    },
    /// No frame has latched the reset-relative start origin.
    #[error("the Aviate runtime has no reset-relative start origin")]
    NoStartOrigin,
    /// An applied condition changed inside a scored window.
    #[error("the applied condition {name} changed inside a scored window")]
    ConditionChanged {
        /// The condition that changed.
        name: String,
    },
    /// The runtime cannot resolve one waveform.
    #[error("the Aviate runtime cannot resolve a {waveform} waveform")]
    UnsupportedWaveform {
        /// The waveform class.
        waveform: &'static str,
    },
    /// A receipt does not name the exact run this runtime admitted.
    #[error("the Aviate {receipt} receipt does not match the exact run intent")]
    ReceiptMismatch {
        /// The receipt class.
        receipt: &'static str,
    },
    /// The admitted mission cannot fly on this runtime.
    #[error("the Aviate runtime refused the mission: {detail}")]
    MissionRejected {
        /// The stable refusal detail.
        detail: String,
    },
    /// The runtime has no admitted run.
    #[error("the Aviate runtime has no admitted run")]
    NoAdmittedRun,
    /// The direct transport refused an operation.
    #[error("the Aviate direct transport refused the operation: {source}")]
    DirectTransport {
        /// The transport failure.
        #[source]
        source: DirectTransportError,
    },
    /// The direct family reached a runtime with no direct authority.
    #[error("the Aviate runtime holds no direct authority for this run")]
    NoDirectAuthority,
    /// A stimulus asks the direct path for a family it does not carry.
    #[error("the Aviate direct path does not carry the {family} family")]
    UnsupportedDirectFamily {
        /// The refused control family.
        family: &'static str,
    },
    /// A direct stimulus does not resolve to one exact physical value.
    #[error("the Aviate direct path cannot resolve the {mapping} mapping exactly")]
    InexactDirectMapping {
        /// The refused mapping rule.
        mapping: &'static str,
    },
    /// A prepared direct command is already open.
    #[error("direct command {sequence} is already prepared and open")]
    DirectIntentOpen {
        /// The open direct command sequence.
        sequence: u64,
    },
    /// A direct result arrived with no open prepared command.
    #[error("the Aviate runtime has no open prepared direct command")]
    NoOpenDirectIntent,
    /// An enacted record does not close the intent it was prepared from.
    #[error("the enacted direct record does not close its prepared intent")]
    DirectRecordMismatch,
    /// A direct command was prepared durably with no durable result.
    #[error("direct command {sequence} has no durable result; the vehicle state is ambiguous")]
    AmbiguousDirectCommand {
        /// The ambiguous direct command sequence.
        sequence: u64,
    },
    /// One direct record cannot become evidence.
    #[error("the Aviate runtime refused to publish a direct record: {detail}")]
    DirectPublicationRejected {
        /// The stable refusal detail.
        detail: &'static str,
    },
    /// Direct evidence does not bind one exact run and runtime.
    #[error("the Aviate direct evidence does not bind its run and runtime")]
    DirectEvidenceUnbound,
    /// A run seal does not bind one exact run and runtime.
    #[error("the Aviate run seal does not bind its run and runtime")]
    UnboundRunSeal,
    /// An executed uncertainty receipt does not answer for its own content.
    #[error("the executed uncertainty receipt is refused: {source}")]
    UnboundUncertaintyReceipt {
        /// The contract failure.
        #[source]
        source: flight_tune::TuneError,
    },
    /// A direct ledger document is larger than its limit.
    #[error("the Aviate direct ledger document is {bytes} bytes")]
    LedgerDocumentSize {
        /// The encoded document size.
        bytes: usize,
    },
    /// A residual direct ledger document already exists.
    #[error("the Aviate direct ledger already holds {name}")]
    DirectLedgerResidual {
        /// The residual document name.
        name: String,
    },
    /// A direct ledger document did not read back unchanged.
    #[error("the Aviate direct ledger document {name} did not read back unchanged")]
    DirectLedgerReadback {
        /// The document name.
        name: String,
    },
    /// Anchored durable storage failed.
    #[error("the Aviate direct ledger failed to {operation}: {source}")]
    Storage {
        /// The storage operation that failed.
        operation: &'static str,
        /// The exact storage failure.
        #[source]
        source: Box<pilotage_durable_storage::StorageError>,
    },
}

impl From<AviateRuntimeError> for AviateActionPortError {
    fn from(error: AviateRuntimeError) -> Self {
        Self::driver("runtime", error.to_string())
    }
}

/// The production Aviate vehicle action driver.
///
/// The driver owns the run's identity, its sample clock, its phase
/// machine, its applied conditions, its direct path, and its seal. The
/// direct path is a type parameter: a runtime built with
/// [`direct::NoDirectControl`] has no sender, no transport, and no durable
/// ledger, so a mission that asks for the direct family is refused rather
/// than quietly shaped through the operator law.
#[derive(Debug)]
pub struct AviateScenarioDriver<D> {
    runtime: AviateRuntimeIdentity,
    capabilities: Vec<MissionCapability>,
    clock: SampleClock,
    phases: PhaseMachine,
    conditions: ConditionLedger,
    direct: D,
    signals: VehicleSignals,
    run: Option<PreparedRun>,
    started: bool,
    seal: Option<AviateRunSeal>,
    executed_uncertainty: Option<flight_tune::ExecutedUncertaintyReceipt>,
}

impl<D: DirectControl> AviateScenarioDriver<D> {
    /// Creates one driver for a sealed runtime identity.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when the runtime does not attest or
    /// the start-state tolerance is unusable.
    pub fn new(
        runtime: AviateRuntimeIdentity,
        capabilities: Vec<MissionCapability>,
        tolerance: StartStateTolerance,
        direct: D,
    ) -> Result<Self, AviateRuntimeError> {
        runtime.attest()?;
        Ok(Self {
            runtime,
            capabilities,
            clock: SampleClock::new(),
            phases: PhaseMachine::new(tolerance)?,
            conditions: ConditionLedger::new(),
            direct,
            signals: VehicleSignals::default(),
            run: None,
            started: false,
            seal: None,
            executed_uncertainty: None,
        })
    }

    /// The sealed production-input identity of this runtime.
    #[must_use]
    pub const fn runtime_identity(&self) -> &AviateRuntimeIdentity {
        &self.runtime
    }

    /// The seal of the last stopped run, when one exists.
    #[must_use]
    pub const fn seal(&self) -> Option<&AviateRunSeal> {
        self.seal.as_ref()
    }

    /// Binds the verified uncertainty this run executed.
    ///
    /// The receipt is bound before the run stops, so a seal cannot be
    /// written for a non-nominal run whose trace path was never verified.
    pub fn bind_executed_uncertainty(
        &mut self,
        receipt: flight_tune::ExecutedUncertaintyReceipt,
    ) -> Result<(), AviateRuntimeError> {
        receipt
            .validate()
            .map_err(|source| AviateRuntimeError::UnboundUncertaintyReceipt { source })?;
        self.executed_uncertainty = Some(receipt);
        Ok(())
    }

    /// The admitted run, when one exists.
    #[must_use]
    pub const fn admitted_run(&self) -> Option<&PreparedRun> {
        self.run.as_ref()
    }

    /// Rejects a restart whose runtime is not the frozen session identity.
    ///
    /// The check runs before any journal, process, socket, simulator, or
    /// vehicle mutation, so a campaign that resumes under a changed
    /// runtime identity stops with nothing external touched.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when the runtime identity differs
    /// from the frozen one.
    pub fn require_frozen_runtime(
        &self,
        frozen: &ArtifactIdentity,
    ) -> Result<(), AviateRuntimeError> {
        self.runtime.require_frozen(frozen)
    }

    fn admitted(&self) -> Result<&PreparedRun, AviateRuntimeError> {
        self.run.as_ref().ok_or(AviateRuntimeError::NoAdmittedRun)
    }

    fn accept(&mut self, frame: &ScenarioFrame) -> Result<FrameStamp, AviateRuntimeError> {
        let stamp = self.clock.accept(FrameStamp {
            source_sequence: frame.source_sequence,
            simulator_time_ns: frame.simulator_time_ns,
            trial_time_ns: frame.trial_time_ns,
        })?;
        self.phases.latch_origin(frame)?;
        self.conditions.observe(frame)?;
        Ok(stamp)
    }

    fn advance(
        &mut self,
        stamp: FrameStamp,
        frame: &ScenarioFrame,
        directive: &AviateVehicleDirective,
    ) -> Result<Option<ReceiptResult>, AviateRuntimeError> {
        self.phases.open(stamp);
        let advance = phase::advance(
            &mut self.phases,
            &mut self.conditions,
            &mut self.direct,
            stamp,
            frame,
            directive,
        )?;
        self.signals.normalized_command = advance.commanded;
        self.signals.channel = advance.channel;
        self.signals.saturated = advance.saturated;
        match advance.progress {
            phase::PhaseProgress::Running => Ok(None),
            phase::PhaseProgress::Complete(result) => {
                self.phases.close();
                Ok(Some(result))
            }
        }
    }

    /// The canonical signals the vehicle port adds to the current frame.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when a commanded value is not finite.
    pub fn vehicle_signals(&self) -> Result<Vec<flight_tune::ObservedSignal>, AviateRuntimeError> {
        self.signals.observed()
    }

    /// The canonical telemetry values this runtime is answerable for.
    ///
    /// A backend merges these into the sample it scores. Every other
    /// canonical field comes from the simulator truth projection, which
    /// [`quality::source_of`] states field by field.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when a commanded value is not finite.
    pub fn canonical_telemetry(
        &self,
    ) -> Result<std::collections::BTreeMap<String, f64>, AviateRuntimeError> {
        self.signals.canonical_values(self.seal.is_some())
    }
}

impl<D: DirectControl> AviateActionDriver for AviateScenarioDriver<D> {
    fn action_port_identity(&self) -> &ArtifactIdentity {
        self.runtime.identity()
    }

    fn capabilities(&self) -> &[MissionCapability] {
        &self.capabilities
    }

    fn prepare_blocking(
        &mut self,
        document: &MissionDocument,
        context: &RunExecutionContext,
    ) -> Result<(), AviateActionPortError> {
        let run = PreparedRun::admit(document, context, &self.runtime)?;
        self.clock = SampleClock::new();
        self.phases.reset();
        self.conditions.clear();
        self.started = false;
        self.seal = None;
        self.run = Some(run);
        Ok(())
    }

    fn start_blocking(&mut self) -> Result<(), AviateActionPortError> {
        let run = self.admitted()?;
        run.require_runtime(&self.runtime)?;
        self.started = true;
        Ok(())
    }

    fn observe_blocking(
        &mut self,
        frame: &ScenarioFrame,
        directive: Option<&AviateVehicleDirective>,
    ) -> Result<Option<ReceiptResult>, AviateActionPortError> {
        if !self.started {
            return Err(AviateRuntimeError::NoAdmittedRun.into());
        }
        self.runtime.attest()?;
        let stamp = self.accept(frame)?;
        let (link_valid, estimator_valid) = telemetry::require_vehicle_states(frame)?;
        self.signals.link_valid = link_valid;
        self.signals.estimator_valid = estimator_valid;
        let Some(directive) = directive else {
            return Ok(None);
        };
        Ok(self.advance(stamp, frame, directive)?)
    }

    fn stop_blocking(
        &mut self,
        context: &mut ScenarioStopContext,
    ) -> Result<(), AviateActionPortError> {
        let run = self.admitted()?;
        run.require_runtime(&self.runtime)?;
        if context.last_source_sequence.is_none() {
            context.last_source_sequence = self.clock.last_source_sequence();
        }
        let closure = RunClosure {
            run_intent_digest: run.run_intent_digest(),
            runtime_identity: run.runtime_identity().clone(),
            accepted_frames: self.clock.accepted(),
            direct_evidence: self.direct.seal(run.runtime_identity())?,
            executed_uncertainty: self.executed_uncertainty.clone(),
        };
        self.seal = Some(terminal::seal(&closure, context)?);
        self.direct.revoke();
        self.started = false;
        Ok(())
    }

    fn cleanup_blocking(&mut self) -> Result<(), AviateActionPortError> {
        self.direct.revoke();
        self.clock = SampleClock::new();
        self.phases.reset();
        self.conditions.clear();
        self.signals = VehicleSignals::default();
        self.started = false;
        self.run = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
