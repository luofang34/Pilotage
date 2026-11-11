//! What the engine hands an in-core client: the session host's
//! connection vocabulary, without a transport behind it.

/// One message the engine asks a client to read.
#[derive(Debug, Clone)]
pub(crate) enum ToConnection {
    /// A length-delimited envelope on the reliable session stream.
    BootstrapMessage {
        /// Encoded length-delimited envelope.
        bytes: Vec<u8>,
    },
    /// A length-delimited envelope on the reliable session-events stream.
    AuthorityMessage(Vec<u8>),
    /// One datagram.
    Datagram {
        /// The encoded datagram payload.
        bytes: Vec<u8>,
    },
    /// The engine closed this client.
    Close,
}
