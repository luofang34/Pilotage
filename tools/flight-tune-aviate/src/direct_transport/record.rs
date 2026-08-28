//! The causal record of one direct command.
//!
//! The record carries the whole causal chain of one command: what the
//! scenario asked for, what the transport calculated, what the sender put
//! on the link, and what the flight controller reported back, with the
//! time of each. The production scenario runtime binds this record to raw
//! samples, the terminal receipt, the trace digest, and the campaign
//! journal; the transport itself owns none of that.

use flight_tune::{ControlChannel, ControlFamily, Digest};
use serde::{Deserialize, Serialize};

use super::port::{DirectSenderIdentity, DirectSetpoint};
use super::step::DirectCommandPurpose;

/// Schema version of the direct command record.
pub const DIRECT_COMMAND_RECORD_SCHEMA_VERSION: u16 = 1;

/// The causal times of one direct command, in simulator nanoseconds.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DirectCommandTimes {
    /// When the transport prepared the request.
    pub requested_at_ns: u64,
    /// When the frame left the process.
    pub transmitted_at_ns: u64,
    /// The raw sample that reported the effective setpoint.
    pub effective_at_ns: u64,
    /// The vehicle estimate time of that same sample.
    pub estimate_at_ns: u64,
    /// The simulator truth time of that same sample.
    pub simulator_truth_at_ns: u64,
}

/// The complete causal record of one direct command.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DirectCommandRecord {
    /// Record schema version.
    pub schema_version: u16,
    /// What this command is for.
    pub purpose: DirectCommandPurpose,
    /// The physical control family that the command commands.
    pub family: ControlFamily,
    /// The control channel that the command moves.
    pub channel: ControlChannel,
    /// The normalized stimulus value.
    pub normalized: f64,
    /// The frozen physical envelope of the normalized range.
    pub envelope_digest: Digest,
    /// The frozen direct baseline that the command was built from.
    pub baseline: DirectSetpoint,
    /// The physical target the transport calculated.
    pub requested: DirectSetpoint,
    /// The setpoint the sender put on the command link.
    pub transmitted: DirectSetpoint,
    /// The setpoint the flight controller reported as active.
    pub effective: DirectSetpoint,
    /// The exact command sender identity.
    pub sender: DirectSenderIdentity,
    /// The raw source sample that carried the effective readback.
    pub effective_sample_sequence: u64,
    /// The causal times of the command.
    pub times: DirectCommandTimes,
    /// The run intent that the command binds to.
    pub run_intent_digest: Digest,
    /// The direct transport that sent the command.
    pub transport_identity_digest: Digest,
}
