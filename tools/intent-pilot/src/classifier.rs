//! The intent classifier port: a child process that speaks JSON lines.
//!
//! The pilot writes one request line and reads one reply line. The model
//! behind the process can change and the pilot does not. The classifier
//! gets the newest operator message, the waypoint legend and the names it
//! may answer with. It gets no vehicle state and no message history,
//! because a small model reads both as instructions.
//!
//! The exchange runs in its own task. One classification takes longer
//! than the host's control-frame staleness bound, and the frame loop must
//! not wait for it.

use std::process::Stdio;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::mpsc;

use crate::error::PilotError;
use crate::scenario::{Arrival, HOME, Intent, Scenario};

/// Model load and shader warm-up time allowed at start, in seconds.
const READY_TIMEOUT_S: u64 = 180;
/// Time allowed for one classification, in seconds.
const REPLY_TIMEOUT_S: u64 = 20;
/// Offsets closer than this ratio to a diagonal read as a diagonal.
const DIAGONAL_RATIO: f64 = 2.0;

#[derive(Debug, Serialize)]
struct Request<'a> {
    message: &'a str,
    targets: &'a [String],
    legend: &'a str,
}

#[derive(Debug, Deserialize)]
struct Ready {
    ready: bool,
    #[serde(default)]
    model: String,
}

#[derive(Debug, Deserialize)]
struct Reply {
    target: String,
    on_arrival: Arrival,
    target_prob: f64,
    on_arrival_prob: f64,
    model_ms: f64,
}

/// One classification, as the run record keeps it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Reading {
    /// The operator message that was classified.
    pub message: String,
    /// The classifier's answer.
    pub intent: Intent,
    /// The classifier's probability for the target.
    pub target_prob: f64,
    /// The classifier's probability for the arrival behaviour.
    pub on_arrival_prob: f64,
    /// Inference time the classifier reports, in milliseconds.
    pub model_ms: f64,
    /// Request-to-reply time the pilot measured, in milliseconds.
    pub round_trip_ms: f64,
}

/// A running classifier process.
pub(crate) struct Classifier {
    child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    targets: Vec<String>,
    legend: String,
    /// The model name the process reported when it became ready.
    pub model: String,
}

impl Classifier {
    /// Starts `command` with the shell and waits for its ready line.
    pub(crate) async fn spawn(command: &str, scenario: &Scenario) -> Result<Self, PilotError> {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|source| PilotError::ClassifierSpawn {
                command: command.to_owned(),
                source,
            })?;
        let missing = |pipe: &str| PilotError::ClassifierReply {
            detail: format!("the child has no {pipe} pipe"),
        };
        let stdin = child.stdin.take().ok_or_else(|| missing("stdin"))?;
        let stdout = child.stdout.take().ok_or_else(|| missing("stdout"))?;
        let mut classifier = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout).lines(),
            targets: scenario.target_names(),
            legend: legend(scenario),
            model: String::new(),
        };
        let line = classifier.read_line("ready", READY_TIMEOUT_S).await?;
        let ready: Ready = parse(&line)?;
        if !ready.ready {
            return Err(PilotError::ClassifierReply {
                detail: format!("first line is not a ready line: {line}"),
            });
        }
        classifier.model = ready.model;
        Ok(classifier)
    }

    /// Classifies one operator message.
    pub(crate) async fn classify(&mut self, message: &str) -> Result<Reading, PilotError> {
        let request = Request {
            message,
            targets: &self.targets,
            legend: &self.legend,
        };
        let mut line =
            serde_json::to_string(&request).map_err(|source| PilotError::ClassifierReply {
                detail: format!("cannot encode the request: {source}"),
            })?;
        line.push('\n');
        let started = Instant::now();
        let write = async {
            self.stdin.write_all(line.as_bytes()).await?;
            self.stdin.flush().await
        };
        write.await.map_err(|source| PilotError::ClassifierIo {
            stage: "request write",
            source,
        })?;
        let reply: Reply = parse(&self.read_line("classify", REPLY_TIMEOUT_S).await?)?;
        // The classifier picks from the names it was given. A name from
        // outside that list means the process is broken, and the pilot
        // must not fly to it.
        if !self.targets.contains(&reply.target) {
            return Err(PilotError::ClassifierReply {
                detail: format!("target {} is not a scenario waypoint", reply.target),
            });
        }
        Ok(Reading {
            message: message.to_owned(),
            intent: Intent {
                target: reply.target,
                on_arrival: reply.on_arrival,
            },
            target_prob: reply.target_prob,
            on_arrival_prob: reply.on_arrival_prob,
            model_ms: reply.model_ms,
            round_trip_ms: started.elapsed().as_secs_f64() * 1000.0,
        })
    }

    /// Runs the exchange loop: messages in, readings out. It ends when
    /// either channel closes, and then stops the child.
    pub(crate) async fn serve(
        mut self,
        mut messages: mpsc::Receiver<String>,
        readings: mpsc::Sender<Result<Reading, PilotError>>,
    ) {
        while let Some(message) = messages.recv().await {
            let reading = self.classify(&message).await;
            if readings.send(reading).await.is_err() {
                break;
            }
        }
        if let Err(error) = self.child.kill().await {
            tracing::warn!(%error, "the classifier process did not stop cleanly");
        }
    }

    async fn read_line(&mut self, stage: &'static str, seconds: u64) -> Result<String, PilotError> {
        let next = tokio::time::timeout(Duration::from_secs(seconds), self.stdout.next_line());
        match next.await {
            Err(_) => Err(PilotError::ClassifierTimeout { stage, seconds }),
            Ok(Err(source)) => Err(PilotError::ClassifierIo { stage, source }),
            Ok(Ok(None)) => Err(PilotError::ClassifierReply {
                detail: format!("the process closed its output during {stage}"),
            }),
            Ok(Ok(Some(line))) => Ok(line),
        }
    }
}

fn parse<'a, T: Deserialize<'a>>(line: &'a str) -> Result<T, PilotError> {
    serde_json::from_str(line).map_err(|source| PilotError::ClassifierReply {
        detail: format!("{source}: {line}"),
    })
}

/// The waypoint legend, in the sentence form the classifier was measured
/// with. Only the distances and directions come from the scenario.
pub(crate) fn legend(scenario: &Scenario) -> String {
    let mut text = format!("WAYPOINTS: {HOME} is the launch point and base.");
    for (name, waypoint) in &scenario.waypoints {
        let distance = waypoint.north_m.hypot(waypoint.east_m).round();
        let direction = compass(waypoint.north_m, waypoint.east_m);
        text.push_str(&format!(" {name} is {distance} m {direction} of {HOME}."));
    }
    text
}

fn compass(north_m: f64, east_m: f64) -> &'static str {
    let (north, east) = (north_m.abs(), east_m.abs());
    let northward = north_m >= 0.0;
    let eastward = east_m >= 0.0;
    if north >= DIAGONAL_RATIO * east {
        if northward { "north" } else { "south" }
    } else if east >= DIAGONAL_RATIO * north {
        if eastward { "east" } else { "west" }
    } else {
        match (northward, eastward) {
            (true, true) => "north-east",
            (true, false) => "north-west",
            (false, true) => "south-east",
            (false, false) => "south-west",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::compass;

    #[test]
    fn cardinal_and_diagonal_directions_are_named() {
        assert_eq!(compass(15.0, 0.0), "north");
        assert_eq!(compass(0.0, -15.0), "west");
        assert_eq!(compass(-10.0, 10.0), "south-east");
        assert_eq!(compass(-15.0, 2.0), "south");
    }
}
