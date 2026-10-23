//! Why the link failed.

/// A link failure.
#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    /// The WebTransport endpoint cannot be built.
    #[error("cannot build the client endpoint")]
    Endpoint(#[source] std::io::Error),
    /// The host refused or dropped the connection attempt.
    #[error("cannot connect to {url}")]
    Connect {
        /// The host URL.
        url: String,
        /// The connection failure.
        #[source]
        source: wtransport::error::ConnectingError,
    },
    /// A stream to the host cannot be opened.
    #[error("cannot open the {stream} stream")]
    Stream {
        /// Which stream.
        stream: &'static str,
        /// The transport failure. It is boxed because the transport has
        /// one error type for each step of opening a stream.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// The host did not complete a connection step in time.
    #[error("the host did not complete `{step}` in {seconds} s")]
    HostTimeout {
        /// The step.
        step: &'static str,
        /// The deadline that passed.
        seconds: u64,
    },
    /// A write to the host failed.
    #[error("the link to the host went down during {stage}")]
    LinkDown {
        /// What was being sent.
        stage: &'static str,
        /// The transport failure.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}
