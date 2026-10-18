//! `MSP_MULTIPLE_MSP`: several v1 replies in one round trip.
//!
//! The request payload lists v1 command identifiers; the reply is each
//! sub-reply as `<len:u8> <payload>` in request order. Betaflight drops
//! the trailing sub-replies that do not fit its reply buffer, so a reply
//! can hold fewer entries than requested.

use super::PayloadError;
use super::cursor::Cursor;

/// Encodes the request payload for `commands`.
///
/// # Errors
///
/// [`PayloadError::TooMany`] when a command does not fit MSP v1.
pub fn encode_multiple_request(commands: &[u16]) -> Result<Vec<u8>, PayloadError> {
    commands
        .iter()
        .map(|&command| {
            u8::try_from(command).map_err(|_| PayloadError::TooMany {
                message: "MSP_MULTIPLE_MSP",
                what: "command identifier value",
                count: usize::from(command),
                limit: usize::from(u8::MAX),
            })
        })
        .collect()
}

/// Splits a reply into `(command, payload)` pairs, matched to `requested`
/// in order. Entries past the reply's end are absent.
///
/// # Errors
///
/// [`PayloadError::Truncated`] when an entry's declared length runs past
/// the reply.
pub fn decode_multiple<'a>(
    requested: &[u16],
    payload: &'a [u8],
) -> Result<Vec<(u16, &'a [u8])>, PayloadError> {
    let mut cursor = Cursor::new("MSP_MULTIPLE_MSP", payload);
    let mut entries = Vec::with_capacity(requested.len());
    for &command in requested {
        if cursor.remaining() == 0 {
            break;
        }
        let len = usize::from(cursor.u8()?);
        entries.push((command, cursor.take(len)?));
    }
    Ok(entries)
}
