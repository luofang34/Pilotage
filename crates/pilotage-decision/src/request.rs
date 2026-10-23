//! The request: a state, the questions about it, and images.

use base64::Engine as _;
use serde_json::{Map, Value, json};
use unicode_normalization::UnicodeNormalization;

/// One question.
#[derive(Debug, Clone, PartialEq)]
pub enum Question {
    /// Pick one of the named options; `criteria` maps each option to what
    /// it means.
    Choice {
        /// What to decide.
        instructions: String,
        /// The options in order: name, then meaning.
        criteria: Vec<(String, String)>,
    },
    /// The probability that a statement is true.
    YesNo {
        /// The statement or question.
        instructions: String,
    },
    /// A level on an ordered scale.
    Score {
        /// What to rate.
        instructions: String,
        /// The levels, lowest first.
        levels: Vec<String>,
    },
}

/// One image, encoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// The media type, such as `image/jpeg`.
    pub media_type: String,
    /// The encoded bytes.
    pub bytes: Vec<u8>,
}

/// A request for judgments.
#[derive(Debug, Clone, PartialEq)]
pub struct DecisionRequest {
    /// What the questions are about, as the caller states it.
    pub state: Value,
    /// The questions by name.
    pub questions: Vec<(String, Question)>,
    /// Images the questions refer to.
    pub images: Vec<Image>,
}

impl DecisionRequest {
    /// The System One request body. With `nfc`, every string, keys
    /// included, is in Unicode normalization form C, which some services
    /// need for stable tokens.
    #[must_use]
    pub fn to_system_one(&self, nfc: bool) -> Value {
        let mut questions = Map::new();
        for (name, question) in &self.questions {
            let body = match question {
                Question::Choice {
                    instructions,
                    criteria,
                } => {
                    let criteria: Map<String, Value> = criteria
                        .iter()
                        .map(|(option, meaning)| (option.clone(), Value::String(meaning.clone())))
                        .collect();
                    json!({"type": "choice", "instructions": instructions, "criteria": criteria})
                }
                Question::YesNo { instructions } => {
                    json!({"type": "noul", "instructions": instructions})
                }
                Question::Score {
                    instructions,
                    levels,
                } => {
                    json!({"type": "score", "instructions": instructions, "criteria": levels})
                }
            };
            questions.insert(name.clone(), body);
        }
        let mut body = json!({"state": self.state, "questions": questions});
        if !self.images.is_empty() {
            let images: Vec<Value> = self
                .images
                .iter()
                .map(|image| {
                    let data = base64::engine::general_purpose::STANDARD.encode(&image.bytes);
                    Value::String(format!("data:{};base64,{data}", image.media_type))
                })
                .collect();
            body["images"] = Value::Array(images);
        }
        if nfc { normalized(body) } else { body }
    }
}

/// `value` with every string in normalization form C.
fn normalized(value: Value) -> Value {
    match value {
        Value::String(text) => Value::String(text.nfc().collect()),
        Value::Array(items) => Value::Array(items.into_iter().map(normalized).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, item)| (key.nfc().collect(), normalized(item)))
                .collect(),
        ),
        other => other,
    }
}
