//! Instructions in a person's words: the shared interpreter
//! ([`pilotage_client_session::mission::Chain`]) turns one into a draft
//! command for this session's lane, and only a confirmation sends it.

use std::time::Duration;

use pilotage_client_session::mission::{Chain, ChainLane, Dispatch, Progress};
use serde_json::Value;

use tokio::time::Instant;

use super::{RECEIPT_TIMEOUT, Session, now_ms};
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
            Ok(instruction) if instruction.steps.iter().any(|s| s.command().is_none()) => self.output.refused(
                "interpreter",
                &format!(
                    "{}; this client has no map to find the area on: draw it in the App, or propose a survey file",
                    instruction.describe()
                ),
            ),
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
        let mut commands = Vec::new();
        for step in &draft.steps {
            let Some(command) = step.command() else {
                self.output
                    .refused("desk", "a step needs an area found on a map");
                return Ok(());
            };
            commands.push((step.lane.clone(), command.clone()));
        }
        // The interpreter offered only this session's lane, so every step names it; each
        // step goes after the lane accepts the one before it.
        let mut dispatch = Dispatch::new(commands);
        let mut deadline = Instant::now() + RECEIPT_TIMEOUT;
        loop {
            match dispatch.poll(|_| Some(&self.desk), now_ms()) {
                Progress::Send { command, .. } => {
                    let Some(request) = self.submit(&command).await? else {
                        return Ok(());
                    };
                    self.output.sent(&request);
                    deadline = Instant::now() + RECEIPT_TIMEOUT;
                }
                Progress::Waiting => {
                    self.step(deadline).await?;
                }
                Progress::Done => return Ok(()),
                Progress::Stopped { lane, reason } => {
                    self.output.refused(
                        "desk",
                        &format!("{lane}: {reason}; the steps after it were not sent"),
                    );
                    return Ok(());
                }
            }
        }
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
