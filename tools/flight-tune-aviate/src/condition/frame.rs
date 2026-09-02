//! The length-prefixed frames the trace path carries.
//!
//! Every frame is a four-byte big-endian length and that many bytes of
//! compact JSON. The launcher answers every frame before the executor sends
//! another, so a refused frame stops the run at the sample that caused it.

use std::io::{Read as _, Write as _};
use std::net::TcpStream;

use serde::Serialize;
use serde::de::DeserializeOwned;

use super::error::AviateConditionError;

/// The largest frame the protocol carries.
pub const MAX_FRAME_BYTES: usize = 65_536;

/// Reads one frame.
///
/// # Errors
///
/// Returns [`AviateConditionError`] when the socket fails, when the stated
/// length is larger than the protocol permits, or when the payload is not
/// the expected document.
pub fn read<T: DeserializeOwned>(stream: &mut TcpStream) -> Result<T, AviateConditionError> {
    let mut length = [0_u8; 4];
    stream
        .read_exact(&mut length)
        .map_err(|source| AviateConditionError::trace("read a frame length", source))?;
    let length = u32::from_be_bytes(length) as usize;
    if length > MAX_FRAME_BYTES {
        return Err(AviateConditionError::FrameTooLarge { bytes: length });
    }
    let mut payload = vec![0_u8; length];
    stream
        .read_exact(&mut payload)
        .map_err(|source| AviateConditionError::trace("read a frame payload", source))?;
    serde_json::from_slice(&payload).map_err(|source| AviateConditionError::Frame { source })
}

/// Writes one frame.
///
/// # Errors
///
/// Returns [`AviateConditionError`] when the document cannot be encoded,
/// when it is larger than the protocol permits, or when the socket fails.
pub fn write<T: Serialize>(stream: &mut TcpStream, frame: &T) -> Result<(), AviateConditionError> {
    let payload =
        serde_json::to_vec(frame).map_err(|source| AviateConditionError::Frame { source })?;
    let length = u32::try_from(payload.len()).map_err(|_| AviateConditionError::FrameTooLarge {
        bytes: payload.len(),
    })?;
    if payload.len() > MAX_FRAME_BYTES {
        return Err(AviateConditionError::FrameTooLarge {
            bytes: payload.len(),
        });
    }
    stream
        .write_all(&length.to_be_bytes())
        .and_then(|()| stream.write_all(&payload))
        .and_then(|()| stream.flush())
        .map_err(|source| AviateConditionError::trace("write a frame", source))
}
