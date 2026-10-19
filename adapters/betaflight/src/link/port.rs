//! Request/response MSP over one byte stream.
//!
//! Betaflight answers every MSP v1 command on the port it arrived on, one
//! at a time and in order, so a request waits for the reply carrying its
//! own command and drops anything else (a late reply to an earlier request
//! that timed out, or CLI text).

use std::time::Duration;

use pilotage_msp::{Decoder, Direction, Frame, Version};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::LinkError;

/// How long a reply may take. Betaflight serves one command per serial
/// tick; at the configured 1 kHz a reply takes a few milliseconds.
pub(crate) const REPLY_TIMEOUT: Duration = Duration::from_millis(250);

/// One MSP connection.
#[derive(Debug)]
pub(crate) struct MspPort<S> {
    stream: S,
    decoder: Decoder,
}

impl<S: AsyncRead + AsyncWrite + Unpin> MspPort<S> {
    pub(crate) fn new(stream: S) -> Self {
        Self {
            stream,
            decoder: Decoder::new(),
        }
    }

    /// Sends `command` with `payload` and returns the reply payload.
    pub(crate) async fn request(
        &mut self,
        command: u16,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, LinkError> {
        self.send(command, payload).await?;
        tokio::time::timeout(REPLY_TIMEOUT, self.reply_to(command))
            .await
            .map_err(|_| LinkError::Timeout { command })?
    }

    /// Sends `command` with `payload` without waiting for the reply.
    pub(crate) async fn send(&mut self, command: u16, payload: Vec<u8>) -> Result<(), LinkError> {
        let bytes = Frame::request(Version::V1, command, payload).encode()?;
        self.stream
            .write_all(&bytes)
            .await
            .map_err(|source| LinkError::Io {
                during: "writing a request",
                source,
            })
    }

    /// Reads until a complete frame arrives and returns every frame that
    /// read completed. Cancel-safe: a cancelled call loses no bytes, since
    /// only a completed read feeds the decoder.
    pub(crate) async fn frames(&mut self) -> Result<Vec<Frame>, LinkError> {
        let mut buffer = [0u8; 1_024];
        loop {
            let len = self
                .stream
                .read(&mut buffer)
                .await
                .map_err(|source| LinkError::Io {
                    during: "reading a reply",
                    source,
                })?;
            if len == 0 {
                return Err(LinkError::Closed);
            }
            let frames: Vec<Frame> = self
                .decoder
                .feed(&buffer[..len])
                .into_iter()
                .filter_map(|decoded| {
                    decoded
                        .inspect_err(|error| tracing::debug!(%error, "dropped a corrupt MSP frame"))
                        .ok()
                })
                .collect();
            if !frames.is_empty() {
                return Ok(frames);
            }
        }
    }

    async fn reply_to(&mut self, command: u16) -> Result<Vec<u8>, LinkError> {
        loop {
            for frame in self.frames().await? {
                if let Some(reply) = reply_payload(frame, command) {
                    return reply;
                }
            }
        }
    }
}

/// The payload of `frame` when it answers `command`; `None` otherwise.
pub(crate) fn reply_payload(frame: Frame, command: u16) -> Option<Result<Vec<u8>, LinkError>> {
    if frame.command != command {
        return None;
    }
    Some(match frame.direction {
        Direction::Error => Err(LinkError::Refused { command }),
        Direction::Response | Direction::Request => Ok(frame.payload),
    })
}
