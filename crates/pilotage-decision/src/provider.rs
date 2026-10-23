//! A provider adapts the shared request and answer shapes to one judgment
//! service: its endpoint, its authentication and its quirks.

use crate::{DecisionError, DecisionRequest, DecisionResponse, answer::Answer};

/// One HTTP call to make: the caller's I/O sends it.
#[derive(Debug, Clone, PartialEq)]
pub struct HttpCall {
    /// The endpoint.
    pub url: String,
    /// Request headers.
    pub headers: Vec<(String, String)>,
    /// The JSON body.
    pub body: Vec<u8>,
}

/// A System One compatible judgment service.
#[derive(Debug, Clone, PartialEq)]
pub struct Provider {
    /// The endpoint, such as `http://127.0.0.1:8761/v1/systemone`.
    pub url: String,
    /// Extra request headers, such as authorization.
    pub headers: Vec<(String, String)>,
    /// Send every string in Unicode normalization form C.
    pub normalize_nfc: bool,
    /// A yes/no probability nearer one half than this floor moves to the
    /// floor on its side, as some services publish their results.
    pub yes_no_floor: Option<f64>,
}

impl Provider {
    /// A System One endpoint, such as `llama-server`'s `/v1/systemone`, with
    /// text normalized and no floor.
    #[must_use]
    pub fn system_one(url: &str) -> Self {
        Self {
            url: url.to_owned(),
            headers: Vec::new(),
            normalize_nfc: true,
            yes_no_floor: None,
        }
    }

    /// Reads a provider from a specification: an endpoint URL followed by
    /// options separated by `;`: `floor=0.801` sets the yes/no floor,
    /// `bearer=TOKEN` adds an authorization header, `nfc=off` sends text
    /// as it is.
    ///
    /// # Errors
    /// [`DecisionError::Malformed`] for an unknown option or a bad value.
    pub fn from_spec(spec: &str) -> Result<Self, DecisionError> {
        let mut parts = spec.split(';').map(str::trim);
        let url = parts
            .next()
            .filter(|url| url.starts_with("http"))
            .ok_or_else(|| {
                DecisionError::Malformed(format!(
                    "the decision provider {spec:?} names no http endpoint"
                ))
            })?;
        let mut provider = Self::system_one(url);
        for option in parts.filter(|part| !part.is_empty()) {
            match option.split_once('=') {
                Some(("floor", value)) => {
                    let floor: f64 = value
                        .parse()
                        .ok()
                        .filter(|f: &f64| (0.5..1.0).contains(f))
                        .ok_or_else(|| DecisionError::Malformed(format!("bad floor {value:?}")))?;
                    provider.yes_no_floor = Some(floor);
                }
                Some(("bearer", token)) => {
                    provider
                        .headers
                        .push(("Authorization".to_owned(), format!("Bearer {token}")));
                }
                Some(("nfc", "off")) => provider.normalize_nfc = false,
                _ => {
                    return Err(DecisionError::Malformed(format!(
                        "unknown decision provider option {option:?}"
                    )));
                }
            }
        }
        Ok(provider)
    }

    /// The call that asks `request`.
    #[must_use]
    pub fn call(&self, request: &DecisionRequest) -> HttpCall {
        let mut headers = vec![("Content-Type".to_owned(), "application/json".to_owned())];
        headers.extend(self.headers.iter().cloned());
        HttpCall {
            url: self.url.clone(),
            headers,
            body: request
                .to_system_one(self.normalize_nfc)
                .to_string()
                .into_bytes(),
        }
    }

    /// Reads the service's answer to `request`.
    ///
    /// # Errors
    /// [`DecisionError::Status`] for an unsuccessful status and
    /// [`DecisionError::Malformed`] for a body that does not answer.
    pub fn read(
        &self,
        request: &DecisionRequest,
        status: u16,
        body: &[u8],
    ) -> Result<DecisionResponse, DecisionError> {
        if !(200..300).contains(&status) {
            return Err(DecisionError::Status {
                url: self.url.clone(),
                status,
                body: String::from_utf8_lossy(body).chars().take(200).collect(),
            });
        }
        let asked: Vec<String> = request
            .questions
            .iter()
            .map(|(name, _)| name.clone())
            .collect();
        let mut response = DecisionResponse::from_system_one(body, &asked)?;
        if let Some(floor) = self.yes_no_floor {
            for answer in response.answers.values_mut() {
                if let Answer::YesNo { probability } = answer
                    && probability.max(1.0 - *probability) < floor
                {
                    *probability = if *probability >= 0.5 {
                        floor
                    } else {
                        1.0 - floor
                    };
                }
            }
        }
        Ok(response)
    }
}
