//! Typed MSP payloads: decoders for the replies an adapter reads and
//! encoders for the commands it sends. Layouts follow Betaflight's
//! `msp.c`; all multi-byte fields are little-endian.

mod arming;
mod config;
mod cursor;
mod modes;
mod multiple;
mod rc;
mod status;
mod telemetry;

pub use arming::{ArmingBlocker, ArmingDisableFlags};
pub use config::{ApiVersion, FailsafeConfig, FailsafeProcedure, decode_features, feature};
pub use modes::{BoxIds, MAX_STEP, ModeFlags, ModeRange, encode_set_mode_range, permanent_id};
pub use multiple::{decode_multiple, encode_multiple_request};
pub use rc::{MAX_CHANNELS, encode_set_raw_rc};
pub use status::Status;
pub use telemetry::{Altitude, Attitude, BatteryState, RawGps};

/// Why a payload did not decode.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PayloadError {
    /// The payload ended before a required field.
    #[error("{message} payload is {len} bytes; it needs at least {needed}")]
    Truncated {
        /// The message being decoded.
        message: &'static str,
        /// The payload length received.
        len: usize,
        /// The length the next field needed.
        needed: usize,
    },
    /// A field holds a value the message does not define.
    #[error("{message} field {field} holds undefined value {value}")]
    UndefinedValue {
        /// The message being decoded.
        message: &'static str,
        /// The field name.
        field: &'static str,
        /// The value received.
        value: u32,
    },
    /// An encoder input is outside what the message can carry.
    #[error("{message} cannot carry {count} {what} (limit {limit})")]
    TooMany {
        /// The message being encoded.
        message: &'static str,
        /// What there were too many of.
        what: &'static str,
        /// How many were given.
        count: usize,
        /// The most the message carries.
        limit: usize,
    },
}

#[cfg(test)]
mod tests;
