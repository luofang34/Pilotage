//! The process port of the model port: a child process that speaks JSON lines.
//!
//! The adapter writes its declaration as its first line. After that it reads
//! one request line and writes one reply line. A reply line with an `error`
//! member is an adapter fault and not a reply.

use std::process::Stdio;
use std::time::{Duration, Instant};

use pilotage_agent::{AdapterDeclaration, ModelReply, ModelRequest};
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

/// Model load and warm-up time allowed at start, in seconds.
const READY_TIMEOUT_S: u64 = 300;
/// Time allowed for one reply, in seconds. An image model on a small machine
/// needs most of it.
const REPLY_TIMEOUT_S: u64 = 120;

/// Every way the exchange with an adapter can fail.
#[derive(Debug, thiserror::Error)]
pub enum ModelProcessError {
    /// The adapter process cannot start.
    #[error("cannot start the model adapter `{command}`")]
    Spawn {
        /// The command line that was tried.
        command: String,
        /// The spawn failure.
        #[source]
        source: std::io::Error,
    },
    /// The pipe to the adapter failed.
    #[error("the model adapter pipe failed during {stage}")]
    Io {
        /// The exchange stage that failed.
        stage: &'static str,
        /// The pipe failure.
        #[source]
        source: std::io::Error,
    },
    /// The adapter gave no line in time.
    #[error("the model adapter gave no {stage} in {seconds} s")]
    Timeout {
        /// The exchange stage that timed out.
        stage: &'static str,
        /// The deadline that passed.
        seconds: u64,
    },
    /// The adapter closed its output.
    #[error("the model adapter closed its output during {stage}")]
    Closed {
        /// The exchange stage.
        stage: &'static str,
    },
    /// A line is not the expected JSON document.
    #[error("the model adapter {stage} is not usable: {line}")]
    Decode {
        /// The exchange stage.
        stage: &'static str,
        /// The line that was read.
        line: String,
        /// The decode failure.
        #[source]
        source: serde_json::Error,
    },
    /// The adapter reported a fault of its own.
    #[error("the model adapter reported a fault: {detail}")]
    AdapterFault {
        /// The adapter's words.
        detail: String,
    },
    /// The adapter declared that it is not ready.
    #[error("the model adapter declared that it is not ready")]
    NotReady,
}

#[derive(Debug, Deserialize)]
struct Fault {
    error: String,
}

/// A running model adapter.
pub struct ModelProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    declaration: AdapterDeclaration,
}

impl ModelProcess {
    /// Starts `command` with the shell and waits for the declaration line.
    pub async fn spawn(command: &str) -> Result<Self, ModelProcessError> {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|source| ModelProcessError::Spawn {
                command: command.to_owned(),
                source,
            })?;
        let closed = ModelProcessError::Closed { stage: "start" };
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(closed);
        };
        let mut stdout = BufReader::new(stdout).lines();
        let line = read_line(&mut stdout, "declaration", READY_TIMEOUT_S).await?;
        let declaration: AdapterDeclaration = decode("declaration", &line)?;
        if !declaration.ready {
            return Err(ModelProcessError::NotReady);
        }
        Ok(Self {
            child,
            stdin,
            stdout,
            declaration,
        })
    }

    /// What the adapter declared at start.
    #[must_use]
    pub const fn declaration(&self) -> &AdapterDeclaration {
        &self.declaration
    }

    /// Sends one request and reads its reply. The second value is the
    /// request-to-reply time in milliseconds, as this process measured it.
    pub async fn ask(
        &mut self,
        request: &ModelRequest,
    ) -> Result<(ModelReply, f64), ModelProcessError> {
        let mut line =
            serde_json::to_string(request).map_err(|source| ModelProcessError::Decode {
                stage: "request",
                line: String::new(),
                source,
            })?;
        line.push('\n');
        let started = Instant::now();
        let write = async {
            self.stdin.write_all(line.as_bytes()).await?;
            self.stdin.flush().await
        };
        write.await.map_err(|source| ModelProcessError::Io {
            stage: "request write",
            source,
        })?;
        let line = read_line(&mut self.stdout, "reply", REPLY_TIMEOUT_S).await?;
        if let Ok(fault) = serde_json::from_str::<Fault>(&line) {
            return Err(ModelProcessError::AdapterFault {
                detail: fault.error,
            });
        }
        let reply = decode("reply", &line)?;
        Ok((reply, started.elapsed().as_secs_f64() * 1000.0))
    }

    /// Stops the adapter process.
    pub async fn stop(mut self) {
        if let Err(error) = self.child.kill().await {
            tracing::warn!(%error, "the model adapter did not stop cleanly");
        }
    }
}

async fn read_line(
    stdout: &mut Lines<BufReader<ChildStdout>>,
    stage: &'static str,
    seconds: u64,
) -> Result<String, ModelProcessError> {
    match tokio::time::timeout(Duration::from_secs(seconds), stdout.next_line()).await {
        Err(_) => Err(ModelProcessError::Timeout { stage, seconds }),
        Ok(Err(source)) => Err(ModelProcessError::Io { stage, source }),
        Ok(Ok(None)) => Err(ModelProcessError::Closed { stage }),
        Ok(Ok(Some(line))) => Ok(line),
    }
}

fn decode<'a, T: Deserialize<'a>>(
    stage: &'static str,
    line: &'a str,
) -> Result<T, ModelProcessError> {
    serde_json::from_str(line).map_err(|source| ModelProcessError::Decode {
        stage,
        line: line.to_owned(),
        source,
    })
}
