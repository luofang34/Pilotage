//! What the engine hands a client to write out: the session host's
//! connection vocabulary. A process host writes each message to the
//! client's transport; the core's own clients read them on the same call.

/// The ADR-0011 message class a datagram belongs to, carried so a failed
/// transport-level send is counted against the right class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatagramClass {
    /// Best-effort telemetry fanned out at tick cadence.
    Telemetry,
    /// A `Pong` reply to a client's `Ping` (RTT probe, ADR-0009).
    Pong,
    /// A `FrameRejected` notice returned to a rejected frame's sender.
    FrameRejected,
}

impl DatagramClass {
    /// Stable label for structured logging and the per-class counter table.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            DatagramClass::Telemetry => "telemetry",
            DatagramClass::Pong => "pong",
            DatagramClass::FrameRejected => "frame_rejected",
        }
    }
}

/// One message the engine asks a client's connection to write out.
#[derive(Debug, Clone)]
pub enum ToConnection {
    /// Bytes to write, length-delimited, on the reliable session stream
    /// (handshake, lease, action, and adapter-enactment replies — never
    /// authority events, which use ADR-0005's session-events stream).
    BootstrapMessage {
        /// Encoded length-delimited envelope.
        bytes: Vec<u8>,
        /// Whether a successful write proves media capability negotiation.
        opens_media: bool,
    },
    /// Bytes to write, length-delimited, on the reliable session-events stream.
    AuthorityMessage(Vec<u8>),
    /// Bytes to send as a single datagram, tagged with the ADR-0011 class a
    /// failed send is counted against.
    Datagram {
        /// The datagram's message class, for per-class drop accounting.
        class: DatagramClass,
        /// The encoded datagram payload.
        bytes: Vec<u8>,
    },
    /// The engine asked this connection to close.
    Close,
}
