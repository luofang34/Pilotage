//! The exact direct command sender port.
//!
//! The transport owns no socket. It borrows the command sender that
//! already carries the vehicle's normal command stream, so every direct
//! command keeps that stream's endpoint, MAVLink source identity, frame
//! sequence, and boot time. A second sender would give the trial a second
//! provenance, and a record could then no longer name which link the
//! flight controller answered.

use flight_tune::Digest;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// One absolute direct attitude and collective-force setpoint.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DirectSetpoint {
    /// Absolute roll setpoint in radians.
    pub roll_rad: f64,
    /// Absolute pitch setpoint in radians.
    pub pitch_rad: f64,
    /// Absolute heading setpoint in radians.
    pub yaw_rad: f64,
    /// Normalized collective force.
    pub collective_force: f64,
}

impl DirectSetpoint {
    /// The four axes in a fixed order, for axis-by-axis comparison.
    #[must_use]
    pub const fn axes(&self) -> [f64; 4] {
        [
            self.roll_rad,
            self.pitch_rad,
            self.yaw_rad,
            self.collective_force,
        ]
    }

    /// Whether every axis is a finite number.
    #[must_use]
    pub fn is_finite(&self) -> bool {
        self.axes().iter().all(|axis| axis.is_finite())
    }

    /// Whether every axis of `other` sits within `tolerance` of this one.
    ///
    /// A non-finite axis on either side never matches.
    #[must_use]
    pub fn matches_within(&self, other: &Self, tolerance: f64) -> bool {
        if !tolerance.is_finite() || tolerance < 0.0 {
            return false;
        }
        self.axes()
            .iter()
            .zip(other.axes())
            .all(|(left, right)| (left - right).abs() <= tolerance)
    }
}

/// The exact command sender that transmitted one direct command.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DirectSenderIdentity {
    /// The command endpoint that every frame is addressed to.
    pub endpoint: String,
    /// The MAVLink system identity of the sender.
    pub system_id: u8,
    /// The MAVLink component identity of the sender.
    pub component_id: u8,
    /// The MAVLink frame sequence of this command.
    pub sequence: u8,
    /// The sender's boot time in milliseconds.
    pub time_boot_ms: u32,
    /// The digest of the exact transmitted frame bytes.
    pub frame_digest: Digest,
}

/// What one exact direct command put on the command link.
#[derive(Clone, Debug, PartialEq)]
pub struct TransmittedDirectCommand {
    /// The setpoint the sender encoded into the frame.
    pub setpoint: DirectSetpoint,
    /// The exact sender identity of the frame.
    pub sender: DirectSenderIdentity,
    /// The sender clock when the frame left the process, nanoseconds.
    pub transmitted_at_ns: u64,
}

/// One raw source sample that reports the flight controller's setpoint.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EffectiveSetpointReport {
    /// The setpoint the flight controller reports as active.
    pub setpoint: DirectSetpoint,
    /// The raw source sample sequence that carries the report.
    pub sample_sequence: u64,
    /// The raw source sample time in nanoseconds.
    pub sample_time_ns: u64,
    /// The vehicle estimate time for the same sample, nanoseconds.
    pub estimate_time_ns: u64,
    /// The simulator truth time for the same sample, nanoseconds.
    pub simulator_truth_time_ns: u64,
}

/// The exact direct command sender for one simulator vehicle.
///
/// An implementation owns the flight-controller command link and the raw
/// sample source that reports the controller's effective setpoint.
pub trait DirectCommandSender {
    /// The command endpoint that every frame is addressed to.
    fn command_endpoint(&self) -> String;

    /// The sender clock, in nanoseconds on the simulator sample grid.
    ///
    /// # Errors
    ///
    /// Returns [`DirectSenderError`] when the clock is unreadable.
    fn now_ns(&mut self) -> Result<u64, DirectSenderError>;

    /// Transmits one exact setpoint and reports what reached the link.
    ///
    /// An implementation must transmit the setpoint unchanged or fail. It
    /// must not clamp, shape, or rate-limit the request.
    ///
    /// # Errors
    ///
    /// Returns [`DirectSenderError`] when the frame did not leave the
    /// process unchanged.
    fn transmit_exact_blocking(
        &mut self,
        setpoint: DirectSetpoint,
    ) -> Result<TransmittedDirectCommand, DirectSenderError>;

    /// The newest flight-controller setpoint report on the raw source.
    ///
    /// Returns `None` when the raw source carries no direct report.
    ///
    /// # Errors
    ///
    /// Returns [`DirectSenderError`] when the raw source is unreadable.
    fn effective_setpoint_blocking(
        &mut self,
    ) -> Result<Option<EffectiveSetpointReport>, DirectSenderError>;

    /// Whether the vehicle is stable enough to freeze a direct baseline.
    ///
    /// # Errors
    ///
    /// Returns [`DirectSenderError`] when stability is not observable.
    fn is_stable_blocking(&mut self) -> Result<bool, DirectSenderError>;
}

/// One exact direct command sender operation failed.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
#[error("the exact direct command sender failed during {operation}: {detail}")]
pub struct DirectSenderError {
    operation: &'static str,
    detail: String,
}

impl DirectSenderError {
    /// Creates a sender error with stable diagnostic text.
    #[must_use]
    pub fn new(operation: &'static str, detail: impl Into<String>) -> Self {
        Self {
            operation,
            detail: detail.into(),
        }
    }

    /// The sender operation that failed.
    #[must_use]
    pub const fn operation(&self) -> &'static str {
        self.operation
    }
}
