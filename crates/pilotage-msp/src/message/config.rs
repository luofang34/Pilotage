//! Configuration replies an adapter checks before it flies.

use super::PayloadError;
use super::cursor::Cursor;

/// `MSP_API_VERSION`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApiVersion {
    /// MSP protocol version.
    pub protocol: u8,
    /// API major version.
    pub major: u8,
    /// API minor version.
    pub minor: u8,
}

impl ApiVersion {
    /// Decodes an `MSP_API_VERSION` payload.
    ///
    /// # Errors
    ///
    /// [`PayloadError::Truncated`] when the payload is short.
    pub fn decode(payload: &[u8]) -> Result<Self, PayloadError> {
        let mut cursor = Cursor::new("MSP_API_VERSION", payload);
        Ok(Self {
            protocol: cursor.u8()?,
            major: cursor.u8()?,
            minor: cursor.u8()?,
        })
    }
}

/// What failsafe stage two does (`failsafeProcedure_e`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailsafeProcedure {
    /// Descend under altitude control and disarm on the ground.
    AutoLanding,
    /// Disarm at once.
    Drop,
    /// Fly home under GPS rescue.
    GpsRescue,
}

/// `MSP_FAILSAFE_CONFIG`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailsafeConfig {
    /// Stage-one guard time before stage two, tenths of a second.
    pub delay_ds: u8,
    /// Auto-landing time limit, seconds.
    pub landing_time_s: u8,
    /// Throttle held during stage one, microseconds.
    pub throttle_us: u16,
    /// The stage-two procedure.
    pub procedure: FailsafeProcedure,
}

impl FailsafeConfig {
    /// Decodes an `MSP_FAILSAFE_CONFIG` payload.
    ///
    /// # Errors
    ///
    /// [`PayloadError::Truncated`] when the payload is short, and
    /// [`PayloadError::UndefinedValue`] for an unknown procedure.
    pub fn decode(payload: &[u8]) -> Result<Self, PayloadError> {
        let mut cursor = Cursor::new("MSP_FAILSAFE_CONFIG", payload);
        let delay_ds = cursor.u8()?;
        let landing_time_s = cursor.u8()?;
        let throttle_us = cursor.u16()?;
        cursor.take(3)?; // switch mode, throttle-low delay
        let procedure = match cursor.u8()? {
            0 => FailsafeProcedure::AutoLanding,
            1 => FailsafeProcedure::Drop,
            2 => FailsafeProcedure::GpsRescue,
            other => {
                return Err(PayloadError::UndefinedValue {
                    message: "MSP_FAILSAFE_CONFIG",
                    field: "procedure",
                    value: u32::from(other),
                });
            }
        };
        Ok(Self {
            delay_ds,
            landing_time_s,
            throttle_us,
            procedure,
        })
    }
}

/// Feature bits (`features_e`) an adapter checks.
pub mod feature {
    /// Receiver channels from the UDP simulator RC link.
    pub const RX_UDP: u32 = 1 << 1;
    /// Receiver channels from a serial receiver.
    pub const RX_SERIAL: u32 = 1 << 3;
    /// GNSS receiver.
    pub const GPS: u32 = 1 << 7;
    /// Receiver channels from `MSP_SET_RAW_RC`.
    pub const RX_MSP: u32 = 1 << 14;
    /// Airmode.
    pub const AIRMODE: u32 = 1 << 22;
}

/// Decodes an `MSP_FEATURE_CONFIG` payload: the enabled feature mask.
///
/// # Errors
///
/// [`PayloadError::Truncated`] when the payload is short.
pub fn decode_features(payload: &[u8]) -> Result<u32, PayloadError> {
    Cursor::new("MSP_FEATURE_CONFIG", payload).u32()
}
