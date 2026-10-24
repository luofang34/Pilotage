//! Agent mode: commands arrive on standard input, one JSON object per
//! line, and are sent without waiting for their receipts. Receipts and
//! status changes go out as they arrive. `{"command": "status"}` prints
//! the lane's full view, with every task's path and evidence.
//! `{"command": "instruct", "text": "..."}`, or for a person any line that
//! is not JSON, interprets an instruction into a draft command, which
//! `{"command": "confirm"}`, or for a person `y`, sends. The session ends
//! when standard input closes.

use pilotage_client_session::mission::MissionCommand;
use tokio::io::{AsyncBufReadExt, BufReader};

use super::Session;
use crate::cli::OperatorError;

impl Session {
    pub(super) async fn agent(&mut self) -> Result<bool, OperatorError> {
        let mut lines = BufReader::new(tokio::io::stdin()).lines();
        loop {
            tokio::select! {
                line = lines.next_line() => match line.map_err(OperatorError::Stdin)? {
                    Some(line) if line.trim().is_empty() => {}
                    Some(line) if is_status(&line) => {
                        match self.desk.view(true, &self.policy) {
                            Some(view) => self.output.full(&view),
                            None => self.output.refused("desk", "the lane has published no status yet"),
                        }
                    }
                    Some(line) if self.instruction_line(&line).await? => {}
                    Some(line) => match serde_json::from_str::<MissionCommand>(&line) {
                        Ok(command) => {
                            if let Some(request) = self.submit(&command).await? {
                                self.output.sent(&request);
                            }
                        }
                        Err(error) => self.output.refused("parser", &error.to_string()),
                    },
                    None => return Ok(true),
                },
                event = self.transport.recv() => match event {
                    Some(event) => {
                        self.on_transport(event).await?;
                    }
                    None => return Err(OperatorError::Host("the link closed".to_owned())),
                },
            }
        }
    }
}

/// A request for the full view: `{"command": "status"}`.
fn is_status(line: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(line)
        .is_ok_and(|value| value.get("command").and_then(|c| c.as_str()) == Some("status"))
}
