//! The frozen direct baseline for one run.
//!
//! A scored step is an offset from a known state, so the state has to be
//! known before the step. The transport enters direct mode, sends one
//! exact baseline as a continuous block until the flight controller reads
//! it back and the vehicle is stable, and then freezes it for the run. A
//! frozen baseline binds to its run intent: another run cannot reuse it.

use flight_tune::Digest;

use super::error::DirectTransportError;
use super::port::DirectSetpoint;

/// What the transport builds a direct baseline from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectBaselineRequest {
    /// The measured attitude, used when no direct setpoint is active.
    pub measured_roll_rad: f64,
    /// The measured pitch attitude in radians.
    pub measured_pitch_rad: f64,
    /// The measured heading in radians.
    pub measured_yaw_rad: f64,
    /// The identified hover trim for neutral collective force.
    pub hover_trim: f64,
    /// The run intent that the frozen baseline binds to.
    pub run_intent_digest: Digest,
    /// The largest number of commands in the baseline block.
    pub max_commands: u32,
}

impl DirectBaselineRequest {
    pub(super) fn validate(&self) -> Result<(), DirectTransportError> {
        for (field, value) in [
            ("measured roll", self.measured_roll_rad),
            ("measured pitch", self.measured_pitch_rad),
            ("measured heading", self.measured_yaw_rad),
            ("hover trim", self.hover_trim),
        ] {
            if !value.is_finite() {
                return Err(DirectTransportError::InvalidValue { field });
            }
        }
        if self.run_intent_digest.is_zero() {
            return Err(DirectTransportError::IncompleteIdentity {
                detail: "the run intent digest is zero".to_owned(),
            });
        }
        if self.max_commands == 0 {
            return Err(DirectTransportError::InvalidValue {
                field: "baseline block length",
            });
        }
        Ok(())
    }

    /// The candidate baseline when no direct setpoint is active.
    pub(super) const fn measured_baseline(&self) -> DirectSetpoint {
        DirectSetpoint {
            roll_rad: self.measured_roll_rad,
            pitch_rad: self.measured_pitch_rad,
            yaw_rad: self.measured_yaw_rad,
            collective_force: self.hover_trim,
        }
    }

    /// The candidate baseline when a direct setpoint is already active.
    ///
    /// The attitude axes come from the effective flight-controller
    /// setpoint. The collective always comes from the identified hover
    /// trim, because the trim is what a vertical stimulus measures from.
    pub(super) const fn effective_baseline(&self, effective: DirectSetpoint) -> DirectSetpoint {
        DirectSetpoint {
            roll_rad: effective.roll_rad,
            pitch_rad: effective.pitch_rad,
            yaw_rad: effective.yaw_rad,
            collective_force: self.hover_trim,
        }
    }
}

/// The frozen direct baseline of one run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectBaseline {
    setpoint: DirectSetpoint,
    hover_trim: f64,
    run_intent_digest: Digest,
    frozen_at_ns: u64,
    commands: u32,
}

impl DirectBaseline {
    pub(super) const fn new(
        setpoint: DirectSetpoint,
        hover_trim: f64,
        run_intent_digest: Digest,
        frozen_at_ns: u64,
        commands: u32,
    ) -> Self {
        Self {
            setpoint,
            hover_trim,
            run_intent_digest,
            frozen_at_ns,
            commands,
        }
    }

    /// The frozen baseline setpoint.
    #[must_use]
    pub const fn setpoint(&self) -> DirectSetpoint {
        self.setpoint
    }

    /// The identified hover trim that the collective baseline came from.
    #[must_use]
    pub const fn hover_trim(&self) -> f64 {
        self.hover_trim
    }

    /// The run intent that this baseline binds to.
    #[must_use]
    pub const fn run_intent_digest(&self) -> Digest {
        self.run_intent_digest
    }

    /// The simulator time at which the baseline froze.
    #[must_use]
    pub const fn frozen_at_ns(&self) -> u64 {
        self.frozen_at_ns
    }

    /// The number of commands the baseline block sent before it froze.
    #[must_use]
    pub const fn commands(&self) -> u32 {
        self.commands
    }

    /// Rejects a run intent that is not the one this baseline froze for.
    ///
    /// # Errors
    ///
    /// Returns [`DirectTransportError`] when the run intent changed.
    pub fn require_run_intent(
        &self,
        run_intent_digest: Digest,
    ) -> Result<(), DirectTransportError> {
        if run_intent_digest == self.run_intent_digest {
            return Ok(());
        }
        Err(DirectTransportError::ChangedPreparedCommand {
            detail: "the frozen run intent",
        })
    }
}
