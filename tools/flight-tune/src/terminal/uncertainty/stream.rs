//! The relation between what one run declared and what it executed.
//!
//! A stream accepts samples in order and derives, for each one, the decision
//! the declaration required. Nothing is taken on the executor's word: the
//! offsets, the digests, the scale, the schedule, and the counts are all
//! derived again here, and a sample that states anything else ends the run.
//!
//! The stream is the only place a ledger is produced, so a count cannot
//! exist without the samples that made it.

use super::super::invalid_terminal;
use super::ledger::ExecutedUncertaintyLedger;
use super::sample::{ExecutedHoverInitialization, ExecutedSample};
use super::{ExecutedUncertaintyDeclaration, derivation};
use crate::{Digest, TuneError};

mod actuator;
mod sensor;

use actuator::ActuatorState;
use sensor::SensorState;

/// What one closed stream states about the run it verified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutedStreamSummary {
    /// The counts the verified samples produced.
    pub ledger: ExecutedUncertaintyLedger,
    /// The identity of the exact ordered samples the counts came from.
    pub sample_stream_digest: Digest,
}

/// One complete stream of executed-uncertainty evidence.
pub struct ExecutedStream<'a> {
    declaration: &'a ExecutedUncertaintyDeclaration,
    ledger: ExecutedUncertaintyLedger,
    sample_stream_digest: Digest,
    sensor: SensorState,
    actuator: ActuatorState,
    hover: Option<ExecutedHoverInitialization>,
    last_sequence: Option<u64>,
    last_global_sample_sequence: Option<u64>,
    last_timestamp_us: Option<u64>,
}

impl<'a> ExecutedStream<'a> {
    /// Opens one stream against the declaration it must answer for.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the declaration is not valid.
    pub fn open(declaration: &'a ExecutedUncertaintyDeclaration) -> Result<Self, TuneError> {
        declaration.validate()?;
        let lane_tags = declaration
            .sensor_lanes
            .iter()
            .map(|lane| lane.lane_tag)
            .collect::<Vec<_>>();
        Ok(Self {
            declaration,
            ledger: ExecutedUncertaintyLedger::opened(&lane_tags),
            sample_stream_digest: derivation::empty_sample_stream(),
            sensor: SensorState::new(),
            actuator: ActuatorState::new(),
            hover: None,
            last_sequence: None,
            last_global_sample_sequence: None,
            last_timestamp_us: None,
        })
    }

    /// Accepts one sample after deriving every decision it states.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the sample repeats, skips, or rewinds a
    /// sequence, when its send is not the one lockstep answer, when the
    /// hover initialization moves, or when any derived value differs from
    /// the stated one.
    pub fn accept(&mut self, sample: &ExecutedSample) -> Result<(), TuneError> {
        sample.validate()?;
        self.advance(sample)?;
        self.require_hover(sample)?;
        require_send(sample)?;
        let drawn = sensor::verify(&mut self.sensor, self.declaration, sample)?;
        let scaled = actuator::verify(&mut self.actuator, self.declaration, sample)?;
        self.sample_stream_digest =
            derivation::extend_sample_stream(self.sample_stream_digest, sample)?;
        self.ledger.count(sample, &drawn, scaled);
        Ok(())
    }

    /// Closes the stream and states the counts its samples produced.
    ///
    /// # Errors
    ///
    /// Returns [`TuneError`] when the stream carried no sample, when the
    /// counts do not answer for it, or when a perturbation is still active.
    pub fn close(self) -> Result<ExecutedStreamSummary, TuneError> {
        if self.actuator.holds_a_command() {
            return Err(invalid_terminal(
                "the stream ended with an active command hold",
            ));
        }
        self.ledger.validate()?;
        Ok(ExecutedStreamSummary {
            ledger: self.ledger,
            sample_stream_digest: self.sample_stream_digest,
        })
    }

    /// Returns the counts folded so far.
    #[must_use]
    pub const fn ledger(&self) -> &ExecutedUncertaintyLedger {
        &self.ledger
    }

    fn advance(&mut self, sample: &ExecutedSample) -> Result<(), TuneError> {
        require_step(self.last_sequence, sample.sequence, "trace sequence")?;
        require_step(
            self.last_global_sample_sequence,
            sample.global_sample_sequence,
            "sample sequence",
        )?;
        if self
            .last_timestamp_us
            .is_some_and(|previous| sample.simulator_timestamp_us < previous)
        {
            return Err(invalid_terminal("a sample rewinds the simulation time"));
        }
        self.last_sequence = Some(sample.sequence);
        self.last_global_sample_sequence = Some(sample.global_sample_sequence);
        self.last_timestamp_us = Some(sample.simulator_timestamp_us);
        Ok(())
    }

    fn require_hover(&mut self, sample: &ExecutedSample) -> Result<(), TuneError> {
        let hover = sample.hover;
        if hover.scale_basis_points != self.declaration.hover_scale_basis_points {
            return Err(invalid_terminal(
                "a sample states another hover force scale than the declared one",
            ));
        }
        if !hover.estimator_disabled {
            return Err(invalid_terminal(
                "a sample states an active online hover estimator",
            ));
        }
        let derived =
            derivation::scaled_hover_force(hover.baseline_force_bits, hover.scale_basis_points);
        if derived != hover.effective_force_bits {
            return Err(invalid_terminal(
                "the hover force does not follow from its baseline and scale",
            ));
        }
        match self.hover {
            Some(first) if first != hover => Err(invalid_terminal(
                "the hover initialization changed inside one run",
            )),
            Some(_) => Ok(()),
            None => {
                self.hover = Some(hover);
                Ok(())
            }
        }
    }
}

/// Requires one sample to carry exactly one completed lockstep answer.
fn require_send(sample: &ExecutedSample) -> Result<(), TuneError> {
    let send = sample.send;
    if !send.attempted || !send.succeeded {
        return Err(invalid_terminal("a sample has no completed actuator send"));
    }
    if !send.lockstep {
        return Err(invalid_terminal(
            "a sample answered outside the sensor lockstep",
        ));
    }
    if send.echoed_timestamp_us != sample.simulator_timestamp_us {
        return Err(invalid_terminal(
            "a sample send answered another sensor sample",
        ));
    }
    Ok(())
}

/// Requires one counter to advance by exactly one step.
fn require_step(previous: Option<u64>, current: u64, name: &'static str) -> Result<(), TuneError> {
    let Some(previous) = previous else {
        return Ok(());
    };
    if current == previous {
        return Err(invalid_terminal(format!("a sample repeats one {name}")));
    }
    if current < previous {
        return Err(invalid_terminal(format!("a sample rewinds the {name}")));
    }
    if current.wrapping_sub(previous) != 1 {
        return Err(invalid_terminal(format!("a sample skips a {name}")));
    }
    Ok(())
}

/// Requires one derived digest to equal the stated one.
fn require_digest(derived: Digest, stated: Digest, detail: &'static str) -> Result<(), TuneError> {
    if derived == stated {
        return Ok(());
    }
    Err(invalid_terminal(detail))
}
