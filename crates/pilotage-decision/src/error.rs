//! Why a judgment could not be obtained.

/// A failed judgment. A caller treats it as no judgment, never as a "no".
#[derive(Debug, thiserror::Error)]
pub enum DecisionError {
    /// The service answered with a status other than success.
    #[error("the decision service at {url} answered status {status}: {body}")]
    Status {
        /// The endpoint.
        url: String,
        /// The HTTP status.
        status: u16,
        /// The start of the body, for diagnosis.
        body: String,
    },
    /// The service could not be reached or did not answer in time.
    #[error("the decision service at {url} could not be reached: {reason}")]
    Transport {
        /// The endpoint.
        url: String,
        /// What the transport reported.
        reason: String,
    },
    /// The answer is not valid JSON or lacks an asked question.
    #[error("the decision answer is malformed: {0}")]
    Malformed(String),
}
