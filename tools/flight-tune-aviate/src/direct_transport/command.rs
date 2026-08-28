//! Freezing the direct baseline, preparing exact commands, enacting them.
//!
//! Preparation and enactment are separate on purpose. A prepared command
//! is complete and self-describing before any datagram can leave the
//! process, and enactment re-derives it from the transport's own frozen
//! state. A target, channel, family, envelope, baseline, or run intent
//! that changed between the two is refused while the command is still a
//! value in memory.

use super::baseline::{DirectBaseline, DirectBaselineRequest};
use super::error::DirectTransportError;
use super::port::{
    DirectCommandSender, DirectSetpoint, EffectiveSetpointReport, TransmittedDirectCommand,
};
use super::readback::ReadbackSelection;
use super::record::{
    DIRECT_COMMAND_RECORD_SCHEMA_VERSION, DirectCommandRecord, DirectCommandTimes,
};
use super::step::{
    DirectCommandPurpose, DirectStepRequest, PreparedDirectCommand, envelope_digest,
    require_direct_family, resolve_exact_target,
};
use super::{DirectEnactment, DirectTransport};

/// How many times an enacted command re-reads a raw source that has not
/// reached the command time yet. A future sample waits; it does not answer.
const READBACK_POLL_LIMIT: u32 = 64;

/// What the raw source can say about one query time.
enum ReadbackOutcome {
    Exact(EffectiveSetpointReport),
    Pending,
    Absent,
}

impl DirectTransport {
    /// Enters direct mode and freezes the direct baseline for this run.
    ///
    /// The baseline comes from the effective flight-controller setpoint
    /// when a direct setpoint is already active, and from the measured
    /// attitude when none is. The collective baseline is always the
    /// identified hover trim. The transport sends that exact baseline as a
    /// continuous block until the flight controller reads it back and the
    /// vehicle is stable, and then freezes it against the run intent.
    ///
    /// # Errors
    ///
    /// Returns [`DirectTransportError`] when the authority is revoked, when
    /// a baseline is already frozen, when the request is incomplete, when
    /// the command endpoint changed, or when the block ends without a
    /// stable matching readback.
    pub fn freeze_baseline_blocking<S: DirectCommandSender + ?Sized>(
        &mut self,
        sender: &mut S,
        request: &DirectBaselineRequest,
    ) -> Result<DirectBaseline, DirectTransportError> {
        self.require_authority()?;
        if self.baseline.is_some() {
            return Err(DirectTransportError::BaselineFrozen);
        }
        request.validate()?;
        self.session.require_endpoint(&sender.command_endpoint())?;
        let candidate = match sender.effective_setpoint_blocking()? {
            Some(report) => request.effective_baseline(report.setpoint),
            None => request.measured_baseline(),
        };
        let mut commands = 0_u32;
        while commands < request.max_commands {
            commands = commands.wrapping_add(1);
            let transmitted = self.transmit_exact(sender, candidate)?;
            let ReadbackOutcome::Exact(report) =
                self.read_back(sender, transmitted.transmitted_at_ns)?
            else {
                continue;
            };
            if report.setpoint.matches_within(&candidate, self.tolerance)
                && sender.is_stable_blocking()?
            {
                let frozen = DirectBaseline::new(
                    candidate,
                    request.hover_trim,
                    request.run_intent_digest,
                    report.sample_time_ns,
                    commands,
                );
                self.baseline = Some(frozen);
                return Ok(frozen);
            }
        }
        Err(DirectTransportError::BaselineNotSettled { commands })
    }

    /// Prepares the scored exact step for one direct stimulus.
    ///
    /// # Errors
    ///
    /// Returns [`DirectTransportError`] when the authority is revoked, when
    /// the family is not the direct attitude and thrust family, when no
    /// baseline is frozen, or when the envelope cannot resolve the value.
    pub fn prepare_step(
        &self,
        stimulus: &DirectStepRequest,
    ) -> Result<PreparedDirectCommand, DirectTransportError> {
        self.prepare(DirectCommandPurpose::Step, stimulus)
    }

    /// Prepares the family-aware release back to the frozen baseline.
    ///
    /// A direct return test releases by sending the frozen direct baseline
    /// as one exact step. Direct mode stays until data collection ends, so
    /// no mode change can be counted as direct-controller response.
    ///
    /// # Errors
    ///
    /// Returns [`DirectTransportError`] when the authority is revoked, when
    /// the family is not the direct attitude and thrust family, or when no
    /// baseline is frozen.
    pub fn prepare_release(
        &self,
        stimulus: &DirectStepRequest,
    ) -> Result<PreparedDirectCommand, DirectTransportError> {
        self.prepare(DirectCommandPurpose::Release, stimulus)
    }

    /// Enacts one prepared direct command.
    ///
    /// The command is re-derived from the transport's own frozen state
    /// before a datagram can leave the process. With no exact raw source
    /// for the command time, nothing is sent and nothing is recorded.
    ///
    /// # Errors
    ///
    /// Returns [`DirectTransportError`] when the authority is revoked, when
    /// the prepared command no longer matches this transport, when the
    /// endpoint changed, or when the transmitted or effective setpoint
    /// leaves the requested target by more than the declared tolerance.
    pub fn enact_blocking<S: DirectCommandSender + ?Sized>(
        &mut self,
        sender: &mut S,
        prepared: &PreparedDirectCommand,
    ) -> Result<DirectEnactment, DirectTransportError> {
        self.require_authority()?;
        self.session.require_endpoint(&sender.command_endpoint())?;
        self.verify_prepared(prepared)?;
        let requested_at_ns = sender.now_ns()?;
        match self.source_for(sender, requested_at_ns)? {
            ReadbackOutcome::Exact(_) => {}
            ReadbackOutcome::Pending => return Ok(DirectEnactment::Pending),
            ReadbackOutcome::Absent => return Ok(DirectEnactment::NoExactSource),
        }
        let transmitted = self.transmit_exact(sender, prepared.requested)?;
        let effective = self.await_readback(sender, transmitted.transmitted_at_ns)?;
        if !effective
            .setpoint
            .matches_within(&transmitted.setpoint, self.tolerance)
        {
            return Err(DirectTransportError::EffectiveTargetMismatch {
                tolerance: self.tolerance,
            });
        }
        Ok(DirectEnactment::Enacted(Box::new(record_for(
            prepared,
            &transmitted,
            &effective,
            requested_at_ns,
        ))))
    }

    fn prepare(
        &self,
        purpose: DirectCommandPurpose,
        stimulus: &DirectStepRequest,
    ) -> Result<PreparedDirectCommand, DirectTransportError> {
        self.require_direct_stimulus(stimulus.family)?;
        let baseline = self.baseline.ok_or(DirectTransportError::NoBaseline)?;
        Ok(PreparedDirectCommand {
            purpose,
            envelope_digest: envelope_digest(&stimulus.envelope)?,
            baseline: baseline.setpoint(),
            requested: target_for(purpose, stimulus, baseline.setpoint())?,
            run_intent_digest: baseline.run_intent_digest(),
            transport_identity_digest: self.session.identity_digest(),
            stimulus: stimulus.clone(),
        })
    }

    fn verify_prepared(
        &self,
        prepared: &PreparedDirectCommand,
    ) -> Result<(), DirectTransportError> {
        if prepared.transport_identity_digest != self.session.identity_digest() {
            return Err(DirectTransportError::ChangedPreparedCommand {
                detail: "the transport identity",
            });
        }
        let baseline = self.baseline.ok_or(DirectTransportError::NoBaseline)?;
        baseline.require_run_intent(prepared.run_intent_digest)?;
        if prepared.baseline != baseline.setpoint() {
            return Err(DirectTransportError::ChangedPreparedCommand {
                detail: "the frozen direct baseline",
            });
        }
        require_direct_family(prepared.stimulus.family)?;
        if envelope_digest(&prepared.stimulus.envelope)? != prepared.envelope_digest {
            return Err(DirectTransportError::ChangedPreparedCommand {
                detail: "the frozen stimulus envelope",
            });
        }
        if target_for(prepared.purpose, &prepared.stimulus, baseline.setpoint())?
            != prepared.requested
        {
            return Err(DirectTransportError::ChangedPreparedCommand {
                detail: "the re-derived physical target",
            });
        }
        Ok(())
    }

    fn transmit_exact<S: DirectCommandSender + ?Sized>(
        &self,
        sender: &mut S,
        requested: DirectSetpoint,
    ) -> Result<TransmittedDirectCommand, DirectTransportError> {
        let transmitted = sender.transmit_exact_blocking(requested)?;
        if !transmitted
            .setpoint
            .matches_within(&requested, self.tolerance)
        {
            return Err(DirectTransportError::TransmittedTargetMismatch {
                tolerance: self.tolerance,
            });
        }
        Ok(transmitted)
    }

    /// The raw source outcome for the one sample after a transmit.
    fn read_back<S: DirectCommandSender + ?Sized>(
        &self,
        sender: &mut S,
        transmitted_at_ns: u64,
    ) -> Result<ReadbackOutcome, DirectTransportError> {
        let query_at_ns = self.readback.next_sample_after(transmitted_at_ns)?;
        self.source_for(sender, query_at_ns)
    }

    /// Waits out a raw source that has not reached the command time.
    fn await_readback<S: DirectCommandSender + ?Sized>(
        &self,
        sender: &mut S,
        transmitted_at_ns: u64,
    ) -> Result<EffectiveSetpointReport, DirectTransportError> {
        let mut polls = 0_u32;
        while polls < READBACK_POLL_LIMIT {
            polls = polls.wrapping_add(1);
            match self.read_back(sender, transmitted_at_ns)? {
                ReadbackOutcome::Exact(report) => return Ok(report),
                ReadbackOutcome::Pending => {}
                ReadbackOutcome::Absent => break,
            }
        }
        Err(DirectTransportError::NoEffectiveReadback)
    }

    fn source_for<S: DirectCommandSender + ?Sized>(
        &self,
        sender: &mut S,
        query_at_ns: u64,
    ) -> Result<ReadbackOutcome, DirectTransportError> {
        let Some(report) = sender.effective_setpoint_blocking()? else {
            return Ok(ReadbackOutcome::Absent);
        };
        Ok(match self.readback.select(query_at_ns, &report)? {
            ReadbackSelection::Exact => ReadbackOutcome::Exact(report),
            ReadbackSelection::Pending => ReadbackOutcome::Pending,
            ReadbackSelection::Absent => ReadbackOutcome::Absent,
        })
    }
}

/// The physical target that one command purpose asks for.
fn target_for(
    purpose: DirectCommandPurpose,
    stimulus: &DirectStepRequest,
    baseline: DirectSetpoint,
) -> Result<DirectSetpoint, DirectTransportError> {
    match purpose {
        DirectCommandPurpose::Step => resolve_exact_target(stimulus, baseline),
        // A baseline command and a release both command the frozen
        // baseline itself, so neither resolves the envelope.
        DirectCommandPurpose::Baseline | DirectCommandPurpose::Release => {
            require_direct_family(stimulus.family)?;
            Ok(baseline)
        }
    }
}

fn record_for(
    prepared: &PreparedDirectCommand,
    transmitted: &TransmittedDirectCommand,
    effective: &EffectiveSetpointReport,
    requested_at_ns: u64,
) -> DirectCommandRecord {
    DirectCommandRecord {
        schema_version: DIRECT_COMMAND_RECORD_SCHEMA_VERSION,
        purpose: prepared.purpose,
        family: prepared.stimulus.family,
        channel: prepared.stimulus.channel,
        normalized: prepared.stimulus.normalized,
        envelope_digest: prepared.envelope_digest,
        baseline: prepared.baseline,
        requested: prepared.requested,
        transmitted: transmitted.setpoint,
        effective: effective.setpoint,
        sender: transmitted.sender.clone(),
        effective_sample_sequence: effective.sample_sequence,
        times: DirectCommandTimes {
            requested_at_ns,
            transmitted_at_ns: transmitted.transmitted_at_ns,
            effective_at_ns: effective.sample_time_ns,
            estimate_at_ns: effective.estimate_time_ns,
            simulator_truth_at_ns: effective.simulator_truth_time_ns,
        },
        run_intent_digest: prepared.run_intent_digest,
        transport_identity_digest: prepared.transport_identity_digest,
    }
}
