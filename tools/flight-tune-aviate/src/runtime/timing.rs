//! The sample clock that orders every Aviate runtime decision.
//!
//! One run reads frames from one source. A repeated, reordered, or
//! backward-stepping frame would let a phase measure a window it never
//! flew, so the clock refuses it before the phase sees it. Every elapsed
//! time a phase reads is measured from a latched entry time on this clock,
//! never from the host wall clock.

use super::AviateRuntimeError;

/// The exact time and ordering of one accepted scenario frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FrameStamp {
    /// The source sample sequence.
    pub source_sequence: u64,
    /// The absolute simulator time in nanoseconds.
    pub simulator_time_ns: u64,
    /// The elapsed trial time in nanoseconds.
    pub trial_time_ns: u64,
}

/// The monotonic sample clock of one run.
#[derive(Clone, Copy, Debug, Default)]
pub struct SampleClock {
    last: Option<FrameStamp>,
    accepted: u64,
}

impl SampleClock {
    /// Creates one clock with no accepted frame.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            last: None,
            accepted: 0,
        }
    }

    /// Accepts one frame and rejects a repeat or a reordering.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when the sequence does not advance,
    /// when either time steps backward, or when the trial time is after the
    /// simulator time.
    pub fn accept(&mut self, stamp: FrameStamp) -> Result<FrameStamp, AviateRuntimeError> {
        if stamp.trial_time_ns > stamp.simulator_time_ns {
            return Err(AviateRuntimeError::FrameOrder {
                detail: "the trial time is after the simulator time",
            });
        }
        if let Some(last) = self.last {
            if stamp.source_sequence <= last.source_sequence {
                return Err(AviateRuntimeError::FrameOrder {
                    detail: "the source sequence does not advance",
                });
            }
            if stamp.simulator_time_ns < last.simulator_time_ns
                || stamp.trial_time_ns < last.trial_time_ns
            {
                return Err(AviateRuntimeError::FrameOrder {
                    detail: "a frame time steps backward",
                });
            }
        }
        self.last = Some(stamp);
        self.accepted = self.accepted.wrapping_add(1);
        Ok(stamp)
    }

    /// The number of frames this clock accepted.
    #[must_use]
    pub const fn accepted(&self) -> u64 {
        self.accepted
    }

    /// The last accepted frame, when the run has one.
    #[must_use]
    pub const fn last(&self) -> Option<FrameStamp> {
        self.last
    }

    /// The last accepted source sequence, when the run has one.
    #[must_use]
    pub fn last_source_sequence(&self) -> Option<u64> {
        self.last.map(|stamp| stamp.source_sequence)
    }
}

/// The elapsed trial time inside one phase.
///
/// A phase latches its entry time on the frame that opens it, so its
/// window is the same length whatever the host was doing.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PhaseClock {
    entered: Option<FrameStamp>,
}

impl PhaseClock {
    /// Creates one clock that no phase has entered.
    #[must_use]
    pub const fn new() -> Self {
        Self { entered: None }
    }

    /// Latches the entry frame of one phase, once.
    ///
    /// A second call keeps the first entry, so a repeated directive on a
    /// later frame cannot restart a window that is already running.
    pub const fn enter(&mut self, stamp: FrameStamp) -> FrameStamp {
        if self.entered.is_none() {
            self.entered = Some(stamp);
        }
        stamp
    }

    /// Whether one phase has latched its entry frame.
    #[must_use]
    pub const fn is_entered(&self) -> bool {
        self.entered.is_some()
    }

    /// The latched entry frame, when the phase has one.
    #[must_use]
    pub const fn entered(&self) -> Option<FrameStamp> {
        self.entered
    }

    /// Clears the latched entry so the next directive opens a new window.
    pub const fn leave(&mut self) {
        self.entered = None;
    }

    /// The elapsed trial nanoseconds since the phase entry frame.
    ///
    /// # Errors
    ///
    /// Returns [`AviateRuntimeError`] when the phase has no entry frame or
    /// when the current time is before it.
    pub fn elapsed_ns(&self, stamp: FrameStamp) -> Result<u64, AviateRuntimeError> {
        let entered = self.entered.ok_or(AviateRuntimeError::FrameOrder {
            detail: "the phase has no entry frame",
        })?;
        stamp
            .trial_time_ns
            .checked_sub(entered.trial_time_ns)
            .ok_or(AviateRuntimeError::FrameOrder {
                detail: "the phase time steps backward",
            })
    }
}
