use std::time::Duration;

use crate::XPlaneTruthSample;

use super::{
    Message, State, XPlaneTrialError, XPlaneTrialSession, invalid_state, receipt_mismatch,
};

const MAX_PENDING_SAMPLES: usize = 4_096;

impl XPlaneTrialSession {
    /// Reads one sample with a timeout for this call only.
    ///
    /// The session restores the prior socket timeout before this method
    /// returns. A timeout does not silently use the listener handshake limit.
    ///
    /// # Errors
    ///
    /// Returns an error for a zero timeout, timeout, transport failure, sample
    /// gap, duplicate, time rewind, or identity change.
    pub fn next_sample_with_timeout_blocking(
        &mut self,
        timeout: Duration,
    ) -> Result<XPlaneTruthSample, XPlaneTrialError> {
        if timeout.is_zero() {
            return receipt_mismatch("the sample read timeout is zero");
        }
        let prior = self
            .reader
            .get_ref()
            .read_timeout()
            .map_err(|source| super::session_io("read prior sample timeout", source))?;
        self.reader
            .get_mut()
            .set_read_timeout(Some(timeout))
            .map_err(|source| super::session_io("set sample timeout", source))?;
        let sample = self.next_sample_blocking();
        let restored = self.reader.get_mut().set_read_timeout(prior);
        match (sample, restored) {
            (result, Ok(())) => result,
            (Ok(_), Err(source)) => Err(super::session_io("restore sample timeout", source)),
            (Err(read_error), Err(source)) => Err(XPlaneTrialError::ReadTimeoutRestore {
                read_error: read_error.to_string(),
                source,
            }),
        }
    }

    /// Reads and validates the next causal truth sample.
    ///
    /// A command receipt can arrive after one or more samples. The session
    /// keeps those samples in causal order until the caller reads them.
    ///
    /// # Errors
    ///
    /// Returns an error for a gap, duplicate, time rewind, or identity change.
    pub fn next_sample_blocking(&mut self) -> Result<XPlaneTruthSample, XPlaneTrialError> {
        if let Some(sample) = self.pending_samples.pop_front() {
            return Ok(sample);
        }
        if !matches!(self.state, State::Active { .. }) {
            return invalid_state("read sample");
        }
        match self.read_message_blocking("read sample")? {
            Message::Sample(sample) => self.accept_sample(sample),
            Message::Error { generation, code } => {
                Err(XPlaneTrialError::CommandRejected { generation, code })
            }
            Message::AircraftChanged | Message::Hello(_) | Message::Active { .. } => {
                receipt_mismatch("the running simulator identity changed")
            }
            Message::Rewind { .. } => receipt_mismatch("the simulator time rewound"),
            _ => receipt_mismatch("expected SAMPLE"),
        }
    }

    pub(super) fn buffer_sample(
        &mut self,
        sample: XPlaneTruthSample,
    ) -> Result<(), XPlaneTrialError> {
        if self.pending_samples.len() >= MAX_PENDING_SAMPLES {
            return receipt_mismatch("the pending sample limit was exceeded");
        }
        let sample = self.accept_sample(sample)?;
        self.pending_samples.push_back(sample);
        Ok(())
    }

    fn accept_sample(
        &mut self,
        sample: XPlaneTruthSample,
    ) -> Result<XPlaneTruthSample, XPlaneTrialError> {
        let State::Active {
            generation,
            reset_generation,
        } = self.state
        else {
            return invalid_state("read sample");
        };
        if sample.generation != generation || sample.reset_generation != reset_generation {
            return receipt_mismatch("sample generation changed");
        }
        let expected_sequence = self.last_sequence.map_or(0, |value| value.wrapping_add(1));
        if sample.sequence != expected_sequence {
            return receipt_mismatch("sample sequence is not contiguous");
        }
        if self
            .last_sim_time_s
            .is_some_and(|prior| sample.sim_time_s <= prior)
        {
            return receipt_mismatch("sample simulator time did not increase");
        }
        self.last_sequence = Some(sample.sequence);
        self.last_sim_time_s = Some(sample.sim_time_s);
        Ok(sample)
    }

    pub(super) fn completed_sample_count(&self) -> u64 {
        self.last_sequence.map_or(0, |value| value.wrapping_add(1))
    }
}
