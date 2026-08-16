//! Transport events the port feeds into the engine.
//!
//! A transport port owns sockets and operating-system I/O and nothing else:
//! it reports what happened, byte-for-byte, and the engine decides what the
//! bytes mean. The same vocabulary serves a QUIC/WebTransport port, a
//! loopback test harness, and a replayed capture.

/// Identity of one host-initiated unidirectional stream, assigned by the
/// transport port. The engine treats it as opaque and stable for the life of
/// the stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StreamId(pub u64);

/// One thing the transport observed, in the order it was observed.
#[derive(Debug, Clone, PartialEq)]
pub enum TransportEvent {
    /// The connection is up and the bootstrap bidirectional stream is open
    /// for writing.
    Connected,
    /// Bytes arrived on the bootstrap bidirectional stream.
    BootstrapReceived(Vec<u8>),
    /// The host opened a unidirectional stream.
    UniStreamOpened(StreamId),
    /// Bytes arrived on a host-initiated unidirectional stream.
    UniStreamReceived(StreamId, Vec<u8>),
    /// A host-initiated unidirectional stream ended.
    UniStreamClosed(StreamId),
    /// One datagram arrived.
    DatagramReceived(Vec<u8>),
    /// The transport failed or closed. The engine decides whether recovery
    /// is worth attempting; the port only reports.
    TransportLost {
        /// Human-readable transport detail, for diagnostics only. Decisions
        /// key on engine state, never on this text.
        detail: String,
    },
}
