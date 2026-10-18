//! MSP v1 and v2 framing: encoding, the streaming decoder, and the CRC.
//!
//! v1: `$ M <dir> <size:u8> <cmd:u8> <payload> <xor>`, where the checksum
//! is the XOR of size, command, and payload. A v1 size of 255 marks a
//! "jumbo" frame whose real size follows the command as a little-endian
//! `u16`. v2: `$ X <dir> <flag:u8> <cmd:u16> <size:u16> <payload> <crc>`,
//! with CRC-8 DVB-S2 over flag through payload.
//!
//! The decoder skips bytes outside a frame, because a flight controller
//! port also carries CLI text and boot output.

/// The largest payload the decoder accepts. Betaflight's own buffers are
/// smaller; a larger declared size is a corrupt header, not a message.
pub const MAX_PAYLOAD: usize = 4_096;

/// Size marker for a v1 jumbo frame.
const JUMBO_SIZE: u8 = 255;

/// MSP framing version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    /// MSP v1: 8-bit command, XOR checksum.
    V1,
    /// MSP v2: 16-bit command, CRC-8 DVB-S2.
    V2,
}

/// Which way a frame travels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// `<`: to the flight controller.
    Request,
    /// `>`: a reply from the flight controller.
    Response,
    /// `!`: the flight controller refused or does not know the command.
    Error,
}

impl Direction {
    fn marker(self) -> u8 {
        match self {
            Self::Request => b'<',
            Self::Response => b'>',
            Self::Error => b'!',
        }
    }

    fn from_marker(byte: u8) -> Option<Self> {
        match byte {
            b'<' => Some(Self::Request),
            b'>' => Some(Self::Response),
            b'!' => Some(Self::Error),
            _ => None,
        }
    }
}

/// One complete, checksum-verified MSP frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Framing version the frame used.
    pub version: Version,
    /// Travel direction.
    pub direction: Direction,
    /// Command identifier.
    pub command: u16,
    /// Payload bytes.
    pub payload: Vec<u8>,
}

/// Why a frame could not be encoded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EncodeError {
    /// MSP v1 carries an 8-bit command.
    #[error("command {command} does not fit an MSP v1 frame (v1 commands are 0-255)")]
    CommandOutOfRange {
        /// The command that did not fit.
        command: u16,
    },
    /// The payload is longer than the framing (or [`MAX_PAYLOAD`]) allows.
    #[error("payload of {len} bytes is too long for {version:?} framing (limit {limit})")]
    PayloadTooLong {
        /// The framing version requested.
        version: Version,
        /// The payload length.
        len: usize,
        /// The largest length that framing accepts here.
        limit: usize,
    },
}

/// Why the decoder dropped a frame.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DecodeError {
    /// The checksum (v1) or CRC (v2) did not match.
    #[error(
        "{version:?} frame for command {command} failed its check: expected {expected:#04x}, got {actual:#04x}"
    )]
    Checksum {
        /// Framing version of the dropped frame.
        version: Version,
        /// Command the header declared.
        command: u16,
        /// The check value computed over the received bytes.
        expected: u8,
        /// The check value the frame carried.
        actual: u8,
    },
    /// The header declared a payload larger than [`MAX_PAYLOAD`].
    #[error("frame for command {command} declares a {declared}-byte payload (limit {MAX_PAYLOAD})")]
    Oversize {
        /// Command the header declared.
        command: u16,
        /// The declared payload size.
        declared: usize,
    },
}

/// One step of the CRC-8 DVB-S2 (polynomial 0xD5) MSP v2 uses.
#[must_use]
pub fn crc8_dvb_s2(crc: u8, byte: u8) -> u8 {
    let mut crc = crc ^ byte;
    for _ in 0..8 {
        crc = if crc & 0x80 != 0 {
            (crc << 1) ^ 0xD5
        } else {
            crc << 1
        };
    }
    crc
}

impl Frame {
    /// A request frame for `command` with `payload`.
    #[must_use]
    pub fn request(version: Version, command: u16, payload: Vec<u8>) -> Self {
        Self {
            version,
            direction: Direction::Request,
            command,
            payload,
        }
    }

    /// Encodes the frame to wire bytes.
    ///
    /// # Errors
    ///
    /// [`EncodeError::CommandOutOfRange`] for a v1 command above 255, and
    /// [`EncodeError::PayloadTooLong`] for a payload the framing cannot
    /// carry (v1 encodes no jumbo frames).
    pub fn encode(&self) -> Result<Vec<u8>, EncodeError> {
        match self.version {
            Version::V1 => self.encode_v1(),
            Version::V2 => self.encode_v2(),
        }
    }

    fn encode_v1(&self) -> Result<Vec<u8>, EncodeError> {
        let command = u8::try_from(self.command).map_err(|_| EncodeError::CommandOutOfRange {
            command: self.command,
        })?;
        let size = u8::try_from(self.payload.len())
            .ok()
            .filter(|&size| size != JUMBO_SIZE)
            .ok_or(EncodeError::PayloadTooLong {
                version: Version::V1,
                len: self.payload.len(),
                limit: usize::from(JUMBO_SIZE - 1),
            })?;
        let mut bytes = Vec::with_capacity(self.payload.len() + 6);
        bytes.extend_from_slice(&[b'$', b'M', self.direction.marker(), size, command]);
        bytes.extend_from_slice(&self.payload);
        let checksum = self.payload.iter().fold(size ^ command, |acc, b| acc ^ b);
        bytes.push(checksum);
        Ok(bytes)
    }

    fn encode_v2(&self) -> Result<Vec<u8>, EncodeError> {
        let size = u16::try_from(self.payload.len())
            .ok()
            .filter(|&size| usize::from(size) <= MAX_PAYLOAD)
            .ok_or(EncodeError::PayloadTooLong {
                version: Version::V2,
                len: self.payload.len(),
                limit: MAX_PAYLOAD,
            })?;
        let mut bytes = Vec::with_capacity(self.payload.len() + 9);
        bytes.extend_from_slice(&[b'$', b'X', self.direction.marker(), 0]);
        bytes.extend_from_slice(&self.command.to_le_bytes());
        bytes.extend_from_slice(&size.to_le_bytes());
        bytes.extend_from_slice(&self.payload);
        let crc = bytes[3..].iter().fold(0, |crc, &b| crc8_dvb_s2(crc, b));
        bytes.push(crc);
        Ok(bytes)
    }
}

/// Where the decoder stands inside a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    Start,
    Direction(Version),
    V1Size(Direction),
    V1Command {
        direction: Direction,
        size: u8,
    },
    V1JumboLow {
        direction: Direction,
        command: u8,
    },
    V1JumboHigh {
        direction: Direction,
        command: u8,
        low: u8,
    },
    V2Flag(Direction),
    V2Header {
        direction: Direction,
        filled: u8,
    },
    Payload(Pending),
    Check(Pending),
}

/// Header facts of the frame being received.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pending {
    version: Version,
    direction: Direction,
    command: u16,
    size: usize,
}

/// A streaming MSP decoder: push bytes, receive complete frames.
#[derive(Debug)]
pub struct Decoder {
    state: State,
    header: [u8; 5],
    payload: Vec<u8>,
    check: u8,
    skipped: u64,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    /// A decoder waiting for the start of a frame.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: State::Idle,
            header: [0; 5],
            payload: Vec::new(),
            check: 0,
            skipped: 0,
        }
    }

    /// Bytes skipped outside any frame (CLI text, boot output, noise).
    #[must_use]
    pub fn skipped(&self) -> u64 {
        self.skipped
    }

    /// Feeds a run of bytes, returning every frame (or drop) completed by it.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Result<Frame, DecodeError>> {
        bytes.iter().filter_map(|&b| self.push(b)).collect()
    }

    /// Feeds one byte. Returns a frame, or the reason one was dropped, when
    /// this byte completes it.
    pub fn push(&mut self, byte: u8) -> Option<Result<Frame, DecodeError>> {
        match self.state {
            State::Idle => {
                if byte == b'$' {
                    self.state = State::Start;
                } else {
                    self.skipped = self.skipped.wrapping_add(1);
                }
                None
            }
            State::Start => {
                self.state = match byte {
                    b'M' => State::Direction(Version::V1),
                    b'X' => State::Direction(Version::V2),
                    b'$' => State::Start,
                    _ => self.resync(),
                };
                None
            }
            State::Direction(version) => {
                self.state = match (Direction::from_marker(byte), version) {
                    (Some(direction), Version::V1) => State::V1Size(direction),
                    (Some(direction), Version::V2) => State::V2Flag(direction),
                    (None, _) => self.resync(),
                };
                None
            }
            _ => self.push_framed(byte),
        }
    }

    fn resync(&mut self) -> State {
        self.skipped = self.skipped.wrapping_add(1);
        State::Idle
    }

    fn push_framed(&mut self, byte: u8) -> Option<Result<Frame, DecodeError>> {
        match self.state {
            State::V1Size(direction) => {
                self.state = State::V1Command {
                    direction,
                    size: byte,
                };
                None
            }
            State::V1Command { direction, size } if size == JUMBO_SIZE => {
                self.state = State::V1JumboLow {
                    direction,
                    command: byte,
                };
                None
            }
            State::V1Command { direction, size } => {
                self.check = size ^ byte;
                self.begin(Version::V1, direction, u16::from(byte), usize::from(size))
            }
            State::V1JumboLow { direction, command } => {
                self.state = State::V1JumboHigh {
                    direction,
                    command,
                    low: byte,
                };
                None
            }
            State::V1JumboHigh {
                direction,
                command,
                low,
            } => {
                self.check = JUMBO_SIZE ^ command ^ low ^ byte;
                let size = usize::from(u16::from_le_bytes([low, byte]));
                self.begin(Version::V1, direction, u16::from(command), size)
            }
            State::V2Flag(direction) => {
                self.header[0] = byte;
                self.state = State::V2Header {
                    direction,
                    filled: 1,
                };
                None
            }
            State::V2Header { direction, filled } => self.push_v2_header(direction, filled, byte),
            State::Payload(pending) => {
                self.payload.push(byte);
                self.check = match pending.version {
                    Version::V1 => self.check ^ byte,
                    Version::V2 => crc8_dvb_s2(self.check, byte),
                };
                if self.payload.len() >= pending.size {
                    self.state = State::Check(pending);
                }
                None
            }
            State::Check(pending) => Some(self.finish(pending, byte)),
            State::Idle | State::Start | State::Direction(_) => None,
        }
    }

    fn push_v2_header(
        &mut self,
        direction: Direction,
        filled: u8,
        byte: u8,
    ) -> Option<Result<Frame, DecodeError>> {
        self.header[usize::from(filled)] = byte;
        if filled < 4 {
            self.state = State::V2Header {
                direction,
                filled: filled + 1,
            };
            return None;
        }
        self.check = self.header.iter().fold(0, |crc, &b| crc8_dvb_s2(crc, b));
        let command = u16::from_le_bytes([self.header[1], self.header[2]]);
        let size = usize::from(u16::from_le_bytes([self.header[3], self.header[4]]));
        self.begin(Version::V2, direction, command, size)
    }

    fn begin(
        &mut self,
        version: Version,
        direction: Direction,
        command: u16,
        size: usize,
    ) -> Option<Result<Frame, DecodeError>> {
        if size > MAX_PAYLOAD {
            self.state = State::Idle;
            return Some(Err(DecodeError::Oversize {
                command,
                declared: size,
            }));
        }
        let pending = Pending {
            version,
            direction,
            command,
            size,
        };
        self.payload = Vec::with_capacity(size);
        self.state = if size == 0 {
            State::Check(pending)
        } else {
            State::Payload(pending)
        };
        None
    }

    fn finish(&mut self, pending: Pending, actual: u8) -> Result<Frame, DecodeError> {
        self.state = State::Idle;
        if actual != self.check {
            return Err(DecodeError::Checksum {
                version: pending.version,
                command: pending.command,
                expected: self.check,
                actual,
            });
        }
        Ok(Frame {
            version: pending.version,
            direction: pending.direction,
            command: pending.command,
            payload: std::mem::take(&mut self.payload),
        })
    }
}

#[cfg(test)]
mod tests;
