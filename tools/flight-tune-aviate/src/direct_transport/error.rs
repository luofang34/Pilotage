//! Typed refusals from the simulator-only direct transport.

use flight_tune::TuneError;

use super::port::DirectSenderError;

/// A simulator-only direct transport operation failed.
///
/// Every variant refuses a command. The transport has no outcome that
/// alters a request and sends it anyway: an altered direct target would be
/// recorded as flight-controller response.
#[derive(Debug, thiserror::Error)]
pub enum DirectTransportError {
    /// The execution target is not a simulator.
    #[error("the direct transport requires a simulator execution target")]
    HardwareTarget,
    /// One binding receipt does not accept the authenticated tuning session.
    #[error("the {binding} binding did not accept the tuning session")]
    UnverifiedBinding {
        /// The binding that failed.
        binding: &'static str,
    },
    /// One bound identity is missing.
    #[error("the direct transport identity is incomplete: {detail}")]
    IncompleteIdentity {
        /// Stable validation detail.
        detail: String,
    },
    /// One bound identity changed after the transport was authorized.
    #[error("the {binding} identity changed after the direct transport was authorized")]
    ChangedBinding {
        /// The binding that changed.
        binding: &'static str,
    },
    /// The transport no longer holds direct authority.
    #[error("the direct transport authority is revoked")]
    Revoked,
    /// The stimulus does not command the direct attitude and thrust family.
    #[error("the direct transport carries no {family} stimulus")]
    UnsupportedFamily {
        /// The refused control family.
        family: String,
    },
    /// The stimulus mapping does not resolve an exact physical value.
    #[error("the direct transport needs an exact stimulus mapping")]
    InexactMapping,
    /// The envelope physics do not match the control channel.
    #[error("the {channel} envelope declares {detail}")]
    EnvelopePhysics {
        /// The control channel.
        channel: String,
        /// The mismatch that the envelope declares.
        detail: String,
    },
    /// The stimulus envelope refused the normalized value.
    #[error("the stimulus envelope refused the normalized value: {source}")]
    Envelope {
        /// The exact envelope failure.
        #[source]
        source: flight_tune::StimulusError,
    },
    /// The transport has no frozen direct baseline.
    #[error("the direct transport has no frozen direct baseline")]
    NoBaseline,
    /// The direct baseline is already frozen for this run.
    #[error("the direct baseline is already frozen for this run")]
    BaselineFrozen,
    /// The direct baseline did not settle inside its command block.
    #[error("the direct baseline did not reach a stable readback in {commands} commands")]
    BaselineNotSettled {
        /// The number of baseline commands the block sent.
        commands: u32,
    },
    /// A prepared command does not match the transport that must enact it.
    #[error("the prepared direct command does not match {detail}")]
    ChangedPreparedCommand {
        /// The part of the transport state that the command left.
        detail: &'static str,
    },
    /// The raw readback sample is not on the simulator sample grid.
    #[error("the raw readback sample time {sample_time_ns} ns is off the {period_ns} ns grid")]
    InvalidReadbackAlignment {
        /// The reported sample time.
        sample_time_ns: u64,
        /// The declared sample period.
        period_ns: u64,
    },
    /// A causal readback bound is not usable.
    #[error("the causal readback bound is not usable: {detail}")]
    InvalidReadbackBound {
        /// Stable validation detail.
        detail: &'static str,
    },
    /// The transmitted setpoint left the requested target.
    #[error("the transmitted setpoint left the requested target by more than {tolerance}")]
    TransmittedTargetMismatch {
        /// The declared numeric tolerance.
        tolerance: f64,
    },
    /// The effective flight-controller setpoint left the transmitted target.
    #[error("the effective setpoint left the transmitted target by more than {tolerance}")]
    EffectiveTargetMismatch {
        /// The declared numeric tolerance.
        tolerance: f64,
    },
    /// One supplied value is not a usable number.
    #[error("the direct transport received an unusable {field} value")]
    InvalidValue {
        /// The field that carries the value.
        field: &'static str,
    },
    /// A canonical document could not be encoded for its digest.
    #[error("the direct transport could not calculate its {artifact} digest: {source}")]
    Digest {
        /// The artifact that has no digest.
        artifact: &'static str,
        /// The exact encoding failure.
        #[source]
        source: serde_json::Error,
    },
    /// One artifact identity is invalid.
    #[error("the direct transport identity is invalid: {source}")]
    InvalidIdentity {
        /// The exact identity failure.
        #[source]
        source: TuneError,
    },
    /// The exact direct command sender failed.
    #[error("the direct command sender failed: {source}")]
    Sender {
        /// The exact sender failure.
        #[source]
        source: DirectSenderError,
    },
}

impl From<DirectSenderError> for DirectTransportError {
    fn from(source: DirectSenderError) -> Self {
        Self::Sender { source }
    }
}
