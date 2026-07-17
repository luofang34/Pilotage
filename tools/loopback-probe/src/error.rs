//! Typed errors for the `loopback-probe` binary.

/// Errors this tool's run loop can produce.
#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    /// The `--capture` file could not be created or its header written.
    #[error("cannot create capture file {path}: {source}")]
    CaptureFile {
        /// The requested capture path.
        path: String,
        /// The underlying I/O failure.
        #[source]
        source: std::io::Error,
    },
    /// Argument parsing failed: an unknown flag or missing/malformed value.
    #[error("usage error: {message}")]
    Usage {
        /// Human-readable description of what was wrong with the arguments.
        message: String,
    },
    /// `--url` did not parse as a URL this tool understands.
    #[error("invalid --url '{url}': {message}")]
    InvalidUrl {
        /// The raw `--url` value supplied.
        url: String,
        /// Why the URL was rejected.
        message: String,
    },
    /// `--url` named a non-loopback host while `--insecure-loopback` was
    /// set; refused per the flag's documented scope (see `cli`).
    #[error(
        "--insecure-loopback refuses non-loopback host '{host}': cert verification would be \
         skipped against a host this tool cannot vouch for"
    )]
    NonLoopbackHost {
        /// The rejected host.
        host: String,
    },
    /// The underlying `hidapi` backend failed to initialize, enumerate, or
    /// open a device.
    #[error("hidapi error: {message}")]
    Hid {
        /// Message from the underlying `hidapi::HidError`.
        message: String,
    },
    /// The `wtransport` client configuration or connection attempt failed.
    #[error("webtransport connect error: {message}")]
    Connect {
        /// Message from the underlying `wtransport` error.
        message: String,
    },
    /// Opening or using a bidirectional stream failed.
    #[error("bidi stream error: {message}")]
    BidiStream {
        /// Message from the underlying `wtransport` error.
        message: String,
    },
    /// Sending a datagram failed.
    #[error("datagram send error: {message}")]
    DatagramSend {
        /// Message from the underlying `wtransport` error.
        message: String,
    },
    /// The host's reply did not match what this tool sent (wrong message
    /// variant, or a denial where a grant was expected).
    #[error("protocol error: {message}")]
    Protocol {
        /// Human-readable description of the mismatch.
        message: String,
    },
    /// Decoding bytes off the wire into a domain message failed.
    #[error("decode error: {source}")]
    Decode {
        /// Underlying `pilotage-protocol` decode error.
        #[source]
        source: pilotage_protocol::DecodeError,
    },
    /// Loading or parsing the RadioMaster Pocket device profile failed.
    #[error("device profile error: {source}")]
    Profile {
        /// Underlying `pilotage-input` profile error.
        #[source]
        source: pilotage_input::ProfileError,
    },
}

impl From<hidapi::HidError> for ProbeError {
    fn from(source: hidapi::HidError) -> Self {
        Self::Hid {
            message: source.to_string(),
        }
    }
}

impl From<pilotage_protocol::DecodeError> for ProbeError {
    fn from(source: pilotage_protocol::DecodeError) -> Self {
        Self::Decode { source }
    }
}

impl From<pilotage_input::ProfileError> for ProbeError {
    fn from(source: pilotage_input::ProfileError) -> Self {
        Self::Profile { source }
    }
}
