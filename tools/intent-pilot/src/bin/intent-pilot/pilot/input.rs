//! Operator messages from the keyboard.

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

/// The word that ends a live run.
const QUIT: &str = "quit";

/// One line from the operator.
pub(crate) enum Line {
    /// An operator message for the model.
    Message(String),
    /// The operator ended the run, or the input closed.
    Quit,
}

/// Starts a task that reads stdin and returns its line queue.
pub(crate) fn operator_lines() -> mpsc::Receiver<Line> {
    let (lines, queue) = mpsc::channel(16);
    tokio::spawn(async move {
        let mut stdin = BufReader::new(tokio::io::stdin()).lines();
        loop {
            let line = match stdin.next_line().await {
                Ok(Some(text)) if text.trim().eq_ignore_ascii_case(QUIT) => Line::Quit,
                Ok(Some(text)) if text.trim().is_empty() => continue,
                Ok(Some(text)) => Line::Message(text.trim().to_owned()),
                // A closed or broken input ends the run the safe way.
                Ok(None) | Err(_) => Line::Quit,
            };
            let done = matches!(line, Line::Quit);
            if lines.send(line).await.is_err() || done {
                return;
            }
        }
    });
    queue
}

/// The next operator line, or a future that never completes when the run has
/// no keyboard input.
pub(crate) async fn next(queue: &mut Option<mpsc::Receiver<Line>>) -> Option<Line> {
    match queue {
        Some(queue) => queue.recv().await,
        None => std::future::pending().await,
    }
}
