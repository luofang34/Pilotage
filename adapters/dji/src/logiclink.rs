//! Bounded framing for the Mini 3 mobile USB connection.
//!
//! Envelope boundaries carry no authentication. DUML checksums must be checked
//! separately. Video bytes have no implied codec, frame boundary, or sample time.

#[cfg(test)]
mod tests;

/// The verified DUML stream identifier.
pub const CONTROL_CHANNEL: u16 = 0x5749;
/// The verified encoded video stream identifier.
pub const VIDEO_CHANNEL: u16 = 0x574a;
const HEADER_BYTES: usize = 8;
const MAX_PAYLOAD_BYTES: usize = u16::MAX as usize;

/// A connection framing error. A new connection must use a new parser.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LogicLinkError {
    /// Nonzero input did not start with the expected envelope signature.
    #[error("invalid DJI LogicLink envelope signature")]
    Signature,
    /// The declared length exceeds the verified mobile profile.
    #[error("DJI LogicLink payload length {0} exceeds 65535 bytes")]
    PayloadLength(u32),
    /// The stream ended inside a header or payload.
    #[error("DJI LogicLink stream ended with {0} buffered bytes")]
    Truncated(usize),
    /// An earlier framing error invalidated this parser.
    #[error("DJI LogicLink parser is invalid; start a new connection")]
    Invalidated,
}

/// One complete envelope borrowed for the duration of the receive callback.
#[derive(Clone, Copy, Debug)]
pub struct LogicLinkFrame<'a> {
    /// Stream identifier. Unknown channels remain opaque.
    pub channel: u16,
    /// Exact stream bytes without envelope headers or zero padding.
    pub payload: &'a [u8],
}

/// Receive counters for one connection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LogicLinkStats {
    /// Complete envelopes, including empty and unknown channels.
    pub envelopes: u64,
    /// Zero padding outside declared envelope payloads.
    pub padding_bytes: u64,
}

/// A streaming parser with a fixed maximum allocation of 65535 payload bytes.
///
/// It accepts zero padding between envelopes, including across USB reads.
/// It never searches past a corrupt header: doing so could promote video bytes
/// into commands. USB read boundaries do not end the logical stream.
pub struct LogicLinkParser {
    header: [u8; HEADER_BYTES],
    header_length: usize,
    payload_length: usize,
    payload: Vec<u8>,
    invalid: bool,
    stats: LogicLinkStats,
}

impl Default for LogicLinkParser {
    fn default() -> Self {
        Self {
            header: [0; HEADER_BYTES],
            header_length: 0,
            payload_length: 0,
            payload: Vec::with_capacity(MAX_PAYLOAD_BYTES),
            invalid: false,
            stats: LogicLinkStats::default(),
        }
    }
}

impl LogicLinkParser {
    /// Construct an empty parser for one connection epoch.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Consume arbitrary receive chunks without allocating an output queue.
    ///
    /// # Errors
    /// Returns the first invalid envelope header and latches the failure.
    pub fn push(
        &mut self,
        mut bytes: &[u8],
        mut receive: impl FnMut(LogicLinkFrame<'_>),
    ) -> Result<(), LogicLinkError> {
        if self.invalid {
            return Err(LogicLinkError::Invalidated);
        }
        while !bytes.is_empty() {
            if self.header_length < HEADER_BYTES {
                if self.header_length == 0 {
                    let padding = bytes.iter().take_while(|byte| **byte == 0).count();
                    self.stats.padding_bytes =
                        self.stats.padding_bytes.wrapping_add(padding as u64);
                    bytes = &bytes[padding..];
                }
                let count = (HEADER_BYTES - self.header_length).min(bytes.len());
                self.header[self.header_length..self.header_length + count]
                    .copy_from_slice(&bytes[..count]);
                self.header_length = self.header_length.wrapping_add(count);
                bytes = &bytes[count..];
                if self.header_length != HEADER_BYTES {
                    break;
                }
                if let Err(error) = self.validate_header() {
                    self.invalid = true;
                    return Err(error);
                }
            }
            let count = (self.payload_length - self.payload.len()).min(bytes.len());
            self.payload.extend_from_slice(&bytes[..count]);
            bytes = &bytes[count..];
            if self.payload.len() == self.payload_length {
                self.stats.envelopes = self.stats.envelopes.wrapping_add(1);
                receive(LogicLinkFrame {
                    channel: u16::from_le_bytes([self.header[2], self.header[3]]),
                    payload: &self.payload,
                });
                self.header_length = 0;
                self.payload.clear();
            }
        }
        Ok(())
    }

    fn validate_header(&mut self) -> Result<(), LogicLinkError> {
        if self.header[..2] != [0x55, 0xcc] {
            return Err(LogicLinkError::Signature);
        }
        let length = u32::from_le_bytes([
            self.header[4],
            self.header[5],
            self.header[6],
            self.header[7],
        ]);
        if u64::from(length) > MAX_PAYLOAD_BYTES as u64 {
            return Err(LogicLinkError::PayloadLength(length));
        }
        self.payload_length = length as usize;
        Ok(())
    }

    /// Check an explicit EOF. This must not be called after each USB read.
    ///
    /// # Errors
    /// Returns an error for a partial envelope or an invalidated stream.
    pub fn finish(&mut self) -> Result<(), LogicLinkError> {
        if self.invalid {
            return Err(LogicLinkError::Invalidated);
        }
        let buffered = self.buffered_bytes();
        if buffered != 0 {
            self.invalid = true;
            return Err(LogicLinkError::Truncated(buffered));
        }
        Ok(())
    }

    /// Input still waiting for an envelope boundary.
    #[must_use]
    pub fn buffered_bytes(&self) -> usize {
        self.header_length + self.payload.len()
    }

    /// Cumulative counters for this connection.
    #[must_use]
    pub fn stats(&self) -> LogicLinkStats {
        self.stats
    }

    pub(crate) fn pending_channel(&self) -> Option<u16> {
        (self.header_length == HEADER_BYTES)
            .then(|| u16::from_le_bytes([self.header[2], self.header[3]]))
    }
}

/// Wrap a validated DUML frame for the shared mobile connection.
#[must_use]
pub fn encode_control(frame: &turret_dji::DumlFrame) -> Vec<u8> {
    encode_duml(frame.raw())
}

pub(crate) fn encode_duml(frame: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(HEADER_BYTES + frame.len());
    bytes.extend_from_slice(&[0x55, 0xcc]);
    bytes.extend_from_slice(&CONTROL_CHANNEL.to_le_bytes());
    bytes.extend_from_slice(&(frame.len() as u32).to_le_bytes());
    bytes.extend_from_slice(frame);
    bytes
}
