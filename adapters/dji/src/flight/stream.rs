//! Paced velocity writes for an independently authorized virtual-stick session.
//!
//! This scheduler does not enable a mode or establish control ownership.
//! Its caller must verify mode feedback and invalidate it on source or lease loss.

use std::time::Duration;

use super::VirtualStickVelocity;

const PERIOD: Duration = Duration::from_millis(50);
const INPUT_LIFETIME: Duration = Duration::from_millis(250);
const STOP_WRITE_BUDGET: Duration = Duration::from_millis(250);

/// The complete control binding. No field is a wildcard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VirtualStickBinding {
    /// One physical attachment incarnation.
    pub attachment: [u8; 16],
    /// The host-admitted control session.
    pub session: u64,
    /// The current control lease generation.
    pub generation: u64,
    /// The independently identified aircraft revision.
    pub model_revision: u64,
}

/// Local write progress, separate from physical aircraft motion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VirtualStickStreamPhase {
    /// Accept fresh input and produce at most twenty writes per second.
    Streaming,
    /// Attempt one zero-velocity write under a fixed deadline.
    Neutralizing,
    /// The zero write completed; the caller must release virtual-stick mode.
    ReleaseRequired,
    /// A write or transport outcome is uncertain. No further bytes are permitted.
    Uncertain,
}

/// A velocity input cannot renew this stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum VirtualStickStreamError {
    /// The input belongs to another attachment, aircraft, or lease.
    #[error("virtual-stick input binding changed")]
    Binding,
    /// The sequence repeats or moves backwards.
    #[error("virtual-stick input sequence is not forward")]
    Sequence,
    /// The original input deadline is expired or exceeds 250 ms.
    #[error("virtual-stick input requires an original deadline within 250 ms")]
    Deadline,
    /// A stopping stream cannot be restarted by another input.
    #[error("virtual-stick stream requires a new mode authorization")]
    Closed,
}

/// One immutable, single-writer command with its original deadline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VirtualStickWrite {
    token: u64,
    binding: VirtualStickBinding,
    value: VirtualStickVelocity,
    sequence: u16,
    send_until: Duration,
    neutralizing: bool,
}

impl VirtualStickWrite {
    /// Encode the complete DUML command. This performs no I/O.
    #[must_use]
    pub fn packet(self) -> [u8; 30] {
        self.value.packet(self.sequence)
    }

    /// The absolute deadline relative to the adapter's monotonic origin.
    #[must_use]
    pub const fn send_until(self) -> Duration {
        self.send_until
    }
}

/// A latest-value scheduler with a single outstanding transport write.
///
/// Construct this only after matching mode acceptance and fresh enabled feedback.
/// A successful transport write is not an aircraft acknowledgement. The caller
/// must disable the mode when this returns `ReleaseRequired`, and must not claim
/// a stop when this returns `Uncertain`.
pub struct VirtualStickStream {
    binding: VirtualStickBinding,
    phase: VirtualStickStreamPhase,
    value: VirtualStickVelocity,
    input_sequence: Option<u32>,
    input_until: Duration,
    due: Duration,
    stop_until: Option<Duration>,
    token: u64,
    pending: Option<VirtualStickWrite>,
}

impl VirtualStickStream {
    /// Start a neutral stream for an independently confirmed mode and lease.
    #[must_use]
    pub fn new(binding: VirtualStickBinding, now: Duration) -> Self {
        Self {
            binding,
            phase: VirtualStickStreamPhase::Streaming,
            value: VirtualStickVelocity::ZERO,
            input_sequence: None,
            input_until: now.saturating_add(INPUT_LIFETIME),
            due: now,
            stop_until: None,
            token: 0,
            pending: None,
        }
    }

    /// Replace the desired velocity without extending any outstanding write.
    ///
    /// # Errors
    /// Rejects expired input, a different binding, replay, or a stopped stream.
    pub fn submit(
        &mut self,
        binding: VirtualStickBinding,
        sequence: u32,
        value: VirtualStickVelocity,
        now: Duration,
        valid_until: Duration,
    ) -> Result<(), VirtualStickStreamError> {
        self.expire(now);
        if binding != self.binding {
            return Err(VirtualStickStreamError::Binding);
        }
        if self.phase != VirtualStickStreamPhase::Streaming {
            return Err(VirtualStickStreamError::Closed);
        }
        if self.input_sequence.is_some_and(|previous| {
            let delta = sequence.wrapping_sub(previous);
            delta == 0 || delta >= 1 << 31
        }) {
            return Err(VirtualStickStreamError::Sequence);
        }
        if valid_until <= now || valid_until - now > INPUT_LIFETIME {
            return Err(VirtualStickStreamError::Deadline);
        }
        self.input_sequence = Some(sequence);
        self.input_until = valid_until;
        self.value = value;
        Ok(())
    }

    /// Return local progress without renewing input or mode authorization.
    #[must_use]
    pub const fn phase(&self) -> VirtualStickStreamPhase {
        self.phase
    }

    /// Latch a stop. Repeated calls cannot extend the neutral write deadline.
    pub fn stop(&mut self, now: Duration) {
        if self.phase == VirtualStickStreamPhase::Streaming {
            self.phase = VirtualStickStreamPhase::Neutralizing;
            self.stop_until = Some(now.saturating_add(STOP_WRITE_BUDGET));
        }
    }

    /// Close the scheduler after source loss, partial write, or an uncertain receipt.
    pub fn transport_lost(&mut self) {
        self.phase = VirtualStickStreamPhase::Uncertain;
        self.pending = None;
    }

    /// Reserve one write. Missed periods are never sent as a catch-up burst.
    ///
    /// A reserved write must be completed or explicitly discarded before this
    /// can return another command. The caller allocates the DUML sequence.
    pub fn next_write(&mut self, now: Duration, sequence: u16) -> Option<VirtualStickWrite> {
        self.expire(now);
        if self.pending.is_some() {
            return None;
        }
        let (value, send_until, neutralizing) = match self.phase {
            VirtualStickStreamPhase::Streaming if now >= self.due => (
                self.value,
                self.input_until.min(now.saturating_add(PERIOD)),
                false,
            ),
            VirtualStickStreamPhase::Neutralizing => {
                (VirtualStickVelocity::ZERO, self.stop_until?, true)
            }
            _ => return None,
        };
        self.token = self.token.wrapping_add(1);
        let write = VirtualStickWrite {
            token: self.token,
            binding: self.binding,
            value,
            sequence,
            send_until,
            neutralizing,
        };
        self.pending = Some(write);
        Some(write)
    }

    /// Recheck the immutable binding and deadline at the transport boundary.
    #[must_use]
    pub fn permits(&self, write: VirtualStickWrite, now: Duration) -> bool {
        self.pending == Some(write)
            && now < write.send_until
            && match self.phase {
                VirtualStickStreamPhase::Streaming => !write.neutralizing && now < self.input_until,
                VirtualStickStreamPhase::Neutralizing => write.neutralizing,
                _ => false,
            }
    }

    /// Report a complete recorded write. Late receipts latch uncertainty.
    pub fn written(&mut self, write: VirtualStickWrite, now: Duration) {
        if !self.permits(write, now) {
            self.transport_lost();
            return;
        }
        self.pending = None;
        if write.neutralizing {
            self.phase = VirtualStickStreamPhase::ReleaseRequired;
        } else {
            self.due = now.saturating_add(PERIOD);
        }
    }

    /// Discard a reserved command only when the transport wrote no bytes.
    /// A partial or uncertain write must call `transport_lost` instead.
    pub fn discard_unwritten(&mut self, write: VirtualStickWrite, now: Duration) {
        if self.pending == Some(write) {
            self.pending = None;
            self.stop(now);
        }
    }

    fn expire(&mut self, now: Duration) {
        if now >= self.input_until {
            self.stop(now);
        }
        if self.phase == VirtualStickStreamPhase::Neutralizing
            && self.stop_until.is_some_and(|until| now >= until)
        {
            self.transport_lost();
        }
    }
}

#[cfg(test)]
mod tests;
