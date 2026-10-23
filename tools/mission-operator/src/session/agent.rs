//! Agent mode: commands arrive on standard input, one JSON object per
//! line, and are sent without waiting for their receipts. Receipts and
//! status changes go out as they arrive. The session ends when standard
//! input closes.

use pilotage_client_session::mission::MissionCommand;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};

use super::{Session, emit};
use crate::cli::OperatorError;

impl Session {
    pub(super) async fn agent(&mut self) -> Result<bool, OperatorError> {
        let mut lines = BufReader::new(tokio::io::stdin()).lines();
        loop {
            tokio::select! {
                line = lines.next_line() => match line.map_err(OperatorError::Stdin)? {
                    Some(line) if line.trim().is_empty() => {}
                    Some(line) => match serde_json::from_str::<MissionCommand>(&line) {
                        Ok(command) => {
                            if let Some(request) = self.submit(&command).await? {
                                emit(&json!({"event": "sent", "request": request}));
                            }
                        }
                        Err(error) => {
                            emit(&json!({"event": "refused", "by": "parser", "reason": error.to_string()}));
                        }
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
