//! Causal selection of the raw sample that reports one effective setpoint.
//!
//! A direct step is scored against the flight controller's own setpoint,
//! so the sample that reports it has to be the sample the command reached.
//! A sample from after the query time cannot answer for the query time: it
//! waits, and is never promoted to the answer. A sample from further back
//! than the maximum skew has lost the causal link and is never substituted
//! either. The direct phase does not fall back to a delayed estimate: with
//! no exact source the transport sends no direct demand and records no
//! step or release marker at all.

use super::error::DirectTransportError;
use super::port::EffectiveSetpointReport;

/// The causal alignment rule for one raw direct readback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CausalReadbackBound {
    sample_period_ns: u64,
    max_skew_ns: u64,
}

/// The outcome of one causal readback selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadbackSelection {
    /// The sample is the exact source for the query time.
    Exact,
    /// No sample has reached the query time. The caller waits.
    Pending,
    /// No exact source exists inside the causal bound.
    Absent,
}

impl CausalReadbackBound {
    /// Creates a causal readback bound.
    ///
    /// # Errors
    ///
    /// Returns [`DirectTransportError`] when the sample period is zero.
    pub const fn new(
        sample_period_ns: u64,
        max_skew_ns: u64,
    ) -> Result<Self, DirectTransportError> {
        if sample_period_ns == 0 {
            return Err(DirectTransportError::InvalidReadbackBound {
                detail: "the simulator sample period is zero",
            });
        }
        Ok(Self {
            sample_period_ns,
            max_skew_ns,
        })
    }

    /// The simulator sample period in nanoseconds.
    #[must_use]
    pub const fn sample_period_ns(&self) -> u64 {
        self.sample_period_ns
    }

    /// The largest permitted skew between a query time and its sample.
    #[must_use]
    pub const fn max_skew_ns(&self) -> u64 {
        self.max_skew_ns
    }

    /// The first sample time strictly after `transmitted_at_ns`.
    ///
    /// This is the one sample in which an exact step must have reached the
    /// flight controller.
    ///
    /// # Errors
    ///
    /// Returns [`DirectTransportError`] when the next sample time would
    /// leave the clock range.
    pub const fn next_sample_after(
        &self,
        transmitted_at_ns: u64,
    ) -> Result<u64, DirectTransportError> {
        let elapsed = transmitted_at_ns % self.sample_period_ns;
        let Some(next) = transmitted_at_ns.checked_add(self.sample_period_ns - elapsed) else {
            return Err(DirectTransportError::InvalidReadbackBound {
                detail: "the next sample time leaves the clock range",
            });
        };
        Ok(next)
    }

    /// Selects one raw sample as the source for `query_at_ns`.
    ///
    /// # Errors
    ///
    /// Returns [`DirectTransportError`] when the sample sequence and the
    /// sample time do not describe the same instant on the sample clock.
    pub fn select(
        &self,
        query_at_ns: u64,
        report: &EffectiveSetpointReport,
    ) -> Result<ReadbackSelection, DirectTransportError> {
        self.require_alignment(report)?;
        // A sample from after the query time answers for a later state of
        // the vehicle. It waits; it is never promoted to this answer.
        if report.sample_time_ns > query_at_ns {
            return Ok(ReadbackSelection::Pending);
        }
        if query_at_ns - report.sample_time_ns <= self.max_skew_ns {
            return Ok(ReadbackSelection::Exact);
        }
        Ok(ReadbackSelection::Absent)
    }

    /// A raw sample's sequence and time must describe the same instant.
    ///
    /// A report whose two clocks disagree cannot be placed against a
    /// command, so it fails closed instead of being placed approximately.
    const fn require_alignment(
        &self,
        report: &EffectiveSetpointReport,
    ) -> Result<(), DirectTransportError> {
        let misaligned = DirectTransportError::InvalidReadbackAlignment {
            sample_time_ns: report.sample_time_ns,
            period_ns: self.sample_period_ns,
        };
        let Some(expected) = report.sample_sequence.checked_mul(self.sample_period_ns) else {
            return Err(misaligned);
        };
        if expected != report.sample_time_ns {
            return Err(misaligned);
        }
        Ok(())
    }
}
