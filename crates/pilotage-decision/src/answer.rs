//! The answer: one typed answer per question, with probabilities.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::DecisionError;

/// One answer.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    /// The chosen option and every option's probability.
    Choice {
        /// The most probable option.
        choice: String,
        /// Each option's probability.
        probabilities: BTreeMap<String, f64>,
        /// How decisive the distribution is, as the service derives it.
        confidence: f64,
    },
    /// The probability that the statement is true.
    YesNo {
        /// The probability, `[0, 1]`.
        probability: f64,
    },
    /// The expected level on the scale and each level's probability.
    Score {
        /// The expected level, `0` for the lowest.
        score: f64,
        /// Each level's probability, by level index.
        probabilities: BTreeMap<usize, f64>,
        /// How decisive the distribution is.
        confidence: f64,
    },
}

/// The answers to one request.
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionResponse {
    /// The model the service named.
    pub model: String,
    /// The answers by question name.
    pub answers: BTreeMap<String, Answer>,
}

impl DecisionResponse {
    /// Reads a System One answer body. Every asked question must have an
    /// answer of the asked kind; a service that leaves one out is
    /// malformed, not a "no".
    ///
    /// # Errors
    /// [`DecisionError::Malformed`] for a body that is not such an answer.
    pub fn from_system_one(body: &[u8], asked: &[String]) -> Result<Self, DecisionError> {
        let value: Value = serde_json::from_slice(body)
            .map_err(|error| DecisionError::Malformed(error.to_string()))?;
        let model = value
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let all = value
            .get("answers")
            .and_then(Value::as_object)
            .ok_or_else(|| DecisionError::Malformed("no answers object".to_owned()))?;
        let mut answers = BTreeMap::new();
        for name in asked {
            let item = all
                .get(name)
                .ok_or_else(|| DecisionError::Malformed(format!("no answer to {name}")))?;
            answers.insert(name.clone(), read(name, item)?);
        }
        Ok(Self { model, answers })
    }

    /// The yes/no probability of `name`, when it was asked as yes/no.
    #[must_use]
    pub fn yes_no(&self, name: &str) -> Option<f64> {
        match self.answers.get(name) {
            Some(Answer::YesNo { probability }) => Some(*probability),
            _ => None,
        }
    }

    /// The chosen option of `name`, when it was asked as a choice.
    #[must_use]
    pub fn choice(&self, name: &str) -> Option<&str> {
        match self.answers.get(name) {
            Some(Answer::Choice { choice, .. }) => Some(choice),
            _ => None,
        }
    }
}

fn read(name: &str, item: &Value) -> Result<Answer, DecisionError> {
    let malformed = |what: &str| DecisionError::Malformed(format!("{name}: {what}"));
    let number = |key: &str| {
        item.get(key)
            .and_then(Value::as_f64)
            .filter(|v| v.is_finite())
    };
    let probabilities = || -> BTreeMap<String, f64> {
        item.get("probabilities")
            .and_then(Value::as_object)
            .map(|map| {
                map.iter()
                    .filter_map(|(k, v)| Some((k.clone(), v.as_f64()?)))
                    .collect()
            })
            .unwrap_or_default()
    };
    match item.get("type").and_then(Value::as_str) {
        Some("noul") => {
            let probability = number("noul").ok_or_else(|| malformed("no yes/no probability"))?;
            Ok(Answer::YesNo {
                probability: probability.clamp(0.0, 1.0),
            })
        }
        Some("choice") => Ok(Answer::Choice {
            choice: item
                .get("choice")
                .and_then(Value::as_str)
                .ok_or_else(|| malformed("no choice"))?
                .to_owned(),
            probabilities: probabilities(),
            confidence: number("confidence").unwrap_or(0.0),
        }),
        Some("score") => Ok(Answer::Score {
            score: number("score").ok_or_else(|| malformed("no score"))?,
            probabilities: probabilities()
                .into_iter()
                .filter_map(|(k, v)| Some((k.parse().ok()?, v)))
                .collect(),
            confidence: number("confidence").unwrap_or(0.0),
        }),
        other => Err(malformed(&format!("unknown answer type {other:?}"))),
    }
}
