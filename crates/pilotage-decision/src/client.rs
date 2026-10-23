//! A blocking HTTP client for the port, for callers that run it on a
//! worker thread.

use std::time::Duration;

use crate::{DecisionError, DecisionRequest, DecisionResponse, Provider};

/// Sends requests to one provider.
#[derive(Debug, Clone)]
pub struct DecisionClient {
    provider: Provider,
    agent: ureq::Agent,
}

impl DecisionClient {
    /// A client whose calls take at most `timeout`.
    #[must_use]
    pub fn new(provider: Provider, timeout: Duration) -> Self {
        Self {
            provider,
            agent: ureq::AgentBuilder::new().timeout(timeout).build(),
        }
    }

    /// The provider.
    #[must_use]
    pub fn provider(&self) -> &Provider {
        &self.provider
    }

    /// Asks `request` and waits for the answer.
    ///
    /// # Errors
    /// Any [`DecisionError`]; a caller treats it as no judgment.
    pub fn decide_blocking(
        &self,
        request: &DecisionRequest,
    ) -> Result<DecisionResponse, DecisionError> {
        let call = self.provider.call(request);
        let mut http = self.agent.post(&call.url);
        for (name, value) in &call.headers {
            http = http.set(name, value);
        }
        let (status, body) = match http.send_bytes(&call.body) {
            Ok(response) => (response.status(), read_body(response, &call.url)?),
            Err(ureq::Error::Status(status, response)) => (status, read_body(response, &call.url)?),
            Err(ureq::Error::Transport(transport)) => {
                return Err(DecisionError::Transport {
                    url: call.url,
                    reason: transport.to_string(),
                });
            }
        };
        self.provider.read(request, status, &body)
    }
}

/// The largest answer body read, bytes.
const MAX_BODY_BYTES: u64 = 1 << 20;

fn read_body(response: ureq::Response, url: &str) -> Result<Vec<u8>, DecisionError> {
    use std::io::Read as _;
    let mut body = Vec::new();
    response
        .into_reader()
        .take(MAX_BODY_BYTES)
        .read_to_end(&mut body)
        .map_err(|error| DecisionError::Transport {
            url: url.to_owned(),
            reason: error.to_string(),
        })?;
    Ok(body)
}
