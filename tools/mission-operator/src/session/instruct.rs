//! Instructions in a person's words: the shared interpreter
//! ([`pilotage_client_session::mission::Chain`]) turns one into a draft
//! command for this session's lane, and only a confirmation sends it.

use std::time::Duration;

use pilotage_client_session::mission::{Chain, ChainLane};
use serde_json::Value;

use super::Session;
use crate::cli::OperatorError;

/// How long the interpreter may take to answer.
const INTERPRET_TIMEOUT: Duration = Duration::from_secs(90);

impl Session {
    /// Handles `line` when it is an instruction or a confirmation; whether
    /// it was one.
    pub(super) async fn instruction_line(&mut self, line: &str) -> Result<bool, OperatorError> {
        let json: Option<Value> = serde_json::from_str(line).ok();
        let command = json
            .as_ref()
            .and_then(|v| v.get("command"))
            .and_then(Value::as_str);
        let text = match (command, json.is_some()) {
            (Some("confirm"), _) => return self.confirm().await.map(|()| true),
            (Some("instruct"), _) => json
                .as_ref()
                .and_then(|v| v.get("text"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            // A person types words, and y to send the draft.
            (_, false) if self.output.human() => {
                if matches!(line.trim(), "y" | "yes" | "send") {
                    return self.confirm().await.map(|()| true);
                }
                line.trim().to_owned()
            }
            _ => return Ok(false),
        };
        self.instruct(&text).await;
        Ok(true)
    }

    async fn instruct(&mut self, text: &str) {
        let Some(url) = self.instruction_model.clone() else {
            self.output
                .refused("interpreter", "no --instruction-model names an interpreter");
            return;
        };
        let Some(lane) = ChainLane::from_desk(&self.desk, true) else {
            self.output
                .refused("interpreter", "the lane has published no status yet");
            return;
        };
        let chain = Chain::new(vec![lane]);
        let request = chain.request(text, &[]);
        let answer = tokio::task::spawn_blocking(move || ask_blocking(&url, &request)).await;
        let response = match answer {
            Ok(Ok(response)) => response,
            Ok(Err(reason)) => return self.output.refused("interpreter", &reason),
            Err(error) => return self.output.refused("interpreter", &error.to_string()),
        };
        match chain.parse(text, &response) {
            Ok(instruction) => {
                self.output.draft(&instruction);
                self.draft = Some(instruction);
            }
            Err(error) => self.output.refused("interpreter", &error.to_string()),
        }
    }

    async fn confirm(&mut self) -> Result<(), OperatorError> {
        let Some(draft) = self.draft.take() else {
            self.output.refused("desk", "no draft to send");
            return Ok(());
        };
        if let Some(request) = self.submit(&draft.command).await? {
            self.output.sent(&request);
        }
        Ok(())
    }
}

/// Posts `request` to the chat endpoint `url` and returns its response.
fn ask_blocking(url: &str, request: &Value) -> Result<Value, String> {
    ureq::post(url)
        .timeout(INTERPRET_TIMEOUT)
        .send_json(request)
        .map_err(|e| e.to_string())?
        .into_json()
        .map_err(|e| e.to_string())
}
