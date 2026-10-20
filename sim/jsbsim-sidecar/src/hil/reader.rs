//! Splits a TCP byte stream into MAVLink 2 frames and keeps the ones the
//! sidecar acts on.

use super::{ActuatorControls, HIL_ACTUATOR_CONTROLS, crc_matches};

/// MAVLink 2 start byte.
const MAGIC_V2: u8 = 0xFD;
/// Header (start through message id) length.
const HEADER: usize = 10;
/// The incompatibility flag for a signed frame.
const SIGNED: u8 = 0x01;
/// A signature's length.
const SIGNATURE: usize = 13;

/// Accumulates stream bytes and yields actuator commands.
#[derive(Debug, Default)]
pub struct FrameReader {
    buffer: Vec<u8>,
    /// Frames dropped for a bad CRC.
    pub crc_errors: u64,
}

impl FrameReader {
    /// Appends received bytes.
    pub fn push(&mut self, bytes: &[u8]) {
        self.buffer.extend_from_slice(bytes);
    }

    /// The next complete `HIL_ACTUATOR_CONTROLS`, skipping every other
    /// frame and any bytes between frames; `None` when no complete one is
    /// buffered yet.
    pub fn next_actuator_controls(&mut self) -> Option<ActuatorControls> {
        loop {
            let start = self.buffer.iter().position(|&b| b == MAGIC_V2);
            let Some(start) = start else {
                self.buffer.clear();
                return None;
            };
            self.buffer.drain(..start);
            let header = self.buffer.get(..HEADER)?;
            let payload_len = usize::from(header[1]);
            let signature = if header[2] & SIGNED != 0 {
                SIGNATURE
            } else {
                0
            };
            let total = HEADER + payload_len + 2 + signature;
            let frame = self.buffer.get(..total)?;
            let id = u32::from(frame[7]) | (u32::from(frame[8]) << 8) | (u32::from(frame[9]) << 16);
            let crc_at = HEADER + payload_len;
            let received = u16::from_le_bytes([frame[crc_at], frame[crc_at + 1]]);
            let controls = (id == HIL_ACTUATOR_CONTROLS.0).then(|| {
                crc_matches(&frame[1..crc_at], HIL_ACTUATOR_CONTROLS.1, received)
                    .then(|| ActuatorControls::decode(&frame[HEADER..crc_at]))
            });
            match controls {
                Some(Some(controls)) => {
                    self.buffer.drain(..total);
                    return Some(controls);
                }
                Some(None) => {
                    self.crc_errors = self.crc_errors.wrapping_add(1);
                    // A corrupt length could swallow good frames; resync
                    // one byte on.
                    self.buffer.drain(..1);
                }
                None => {
                    self.buffer.drain(..total);
                }
            }
        }
    }
}
