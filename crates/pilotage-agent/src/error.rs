//! Typed errors of the agent core.

/// A document that the agent core cannot use.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    /// The text is not a document of the expected kind.
    #[error("cannot parse the {document}")]
    Parse {
        /// Which document.
        document: &'static str,
        /// The decode failure.
        #[source]
        source: serde_json::Error,
    },
    /// The document breaks one of its own rules.
    #[error("invalid {document}: {detail}")]
    Invalid {
        /// Which document.
        document: &'static str,
        /// The broken rule.
        detail: String,
    },
}
