use pilotage_data_packages::PackageError;

/// An update could not be completed.
#[derive(Debug, thiserror::Error)]
pub enum DeliveryError {
    /// A package failed verification or installation.
    #[error(transparent)]
    Package(#[from] PackageError),
    /// An HTTP request failed.
    #[error("data request failed at {url}")]
    Http {
        /// Requested URL.
        url: String,
        /// Transport error.
        #[source]
        source: reqwest::Error,
    },
    /// A response stream could not be read.
    #[error("data response could not be read at {url}")]
    Read {
        /// Requested URL.
        url: String,
        /// Stream error.
        #[source]
        source: std::io::Error,
    },
    /// Metadata is not valid JSON.
    #[error("catalog JSON could not be decoded")]
    Json(#[from] serde_json::Error),
    /// A URL could not be parsed or used.
    #[error("invalid data URL: {url}")]
    Url {
        /// Rejected URL.
        url: String,
    },
    /// A configured URL has invalid syntax.
    #[error("data URL could not be parsed: {url}")]
    ParseUrl {
        /// Rejected URL.
        url: String,
        /// URL parser error.
        #[source]
        source: url::ParseError,
    },
    /// A response does not match the requested object.
    #[error("invalid data response at {url}: {reason}")]
    Response {
        /// Requested URL.
        url: String,
        /// Rejected response property.
        reason: String,
    },
    /// The host cancelled the operation.
    #[error("data download was cancelled")]
    Cancelled,
    /// No accepted catalog is available for the publisher.
    #[error("no catalog is available for {publisher}")]
    NoCatalog {
        /// Publisher ID.
        publisher: String,
    },
}

pub(crate) fn http_error(url: &str, source: reqwest::Error) -> DeliveryError {
    DeliveryError::Http {
        url: url.to_owned(),
        source,
    }
}

pub(crate) fn response_error(url: &str, reason: impl ToString) -> DeliveryError {
    DeliveryError::Response {
        url: url.to_owned(),
        reason: reason.to_string(),
    }
}
