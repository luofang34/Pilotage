//! `MSP_SET_RAW_RC`: receiver channels for the MSP receiver.

use super::PayloadError;

/// The most channels Betaflight's MSP receiver accepts
/// (`MAX_SUPPORTED_RC_CHANNEL_COUNT`).
pub const MAX_CHANNELS: usize = 18;

/// Encodes channel pulses (microseconds, in the receiver's channel order:
/// by default roll, pitch, throttle, yaw, then aux 1..).
///
/// # Errors
///
/// [`PayloadError::TooMany`] for more than [`MAX_CHANNELS`] channels.
pub fn encode_set_raw_rc(channels: &[u16]) -> Result<Vec<u8>, PayloadError> {
    if channels.len() > MAX_CHANNELS {
        return Err(PayloadError::TooMany {
            message: "MSP_SET_RAW_RC",
            what: "channels",
            count: channels.len(),
            limit: MAX_CHANNELS,
        });
    }
    Ok(channels.iter().flat_map(|us| us.to_le_bytes()).collect())
}
