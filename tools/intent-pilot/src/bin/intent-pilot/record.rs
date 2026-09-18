//! The run record: one JSON object on each line, in event sequence.
//!
//! The record keeps each model reply next to the directive that the scenario
//! author wrote, so a reader sees each reading graded. The verifier's report
//! is the last line of a scenario run.

use std::path::{Path, PathBuf};

use pilotage_agent::{AdapterDeclaration, Directive, ModelReply, Phase, Report, Scenario};
use serde::Serialize;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use crate::error::PilotError;

/// Where an operator message came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Source {
    /// The scenario script.
    Script,
    /// A person at the keyboard.
    Operator,
}

/// One line of the run record.
#[derive(Debug, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub(crate) enum Entry<'a> {
    /// The scenario and the identities of the two ends.
    Start {
        /// The scenario document, complete.
        scenario: &'a Scenario,
        /// What the model adapter declared.
        adapter: &'a AdapterDeclaration,
        /// The legend text that the model receives.
        legend: &'a str,
        /// The control-source identity announced to the host.
        profile_id: &'a str,
        /// True for a run with operator messages from the keyboard.
        live: bool,
    },
    /// An operator message that the model read and the executor took.
    Message {
        /// Seconds since the first message.
        at_s: f64,
        /// The operator message.
        text: &'a str,
        /// Where the message came from.
        source: Source,
        /// The model reply.
        reply: &'a ModelReply,
        /// Request-to-reply time that the pilot measured, in milliseconds.
        round_trip_ms: f64,
        /// The directive that the scenario author wrote for this message.
        means: Option<&'a Directive>,
        /// True when the reply equals the author's directive.
        read_correctly: Option<bool>,
    },
    /// A reply that was not flown. The vehicle keeps its last directive.
    Refused {
        /// Seconds since the first message.
        at_s: f64,
        /// The operator message.
        text: &'a str,
        /// The model reply.
        reply: &'a ModelReply,
        /// Why it was refused.
        reason: String,
        /// The directive that the scenario author wrote for this message.
        means: Option<&'a Directive>,
    },
    /// The model adapter failed on a message. The vehicle keeps its last
    /// directive.
    ModelFault {
        /// Seconds since the first message.
        at_s: f64,
        /// The operator message.
        text: &'a str,
        /// The failure, in words.
        detail: String,
    },
    /// The executor changed phase.
    Phase {
        /// Seconds since the first message.
        at_s: f64,
        /// The new phase.
        phase: Phase,
        /// Estimated position north of the launch point, in metres.
        north_m: f64,
        /// Estimated position east of the launch point, in metres.
        east_m: f64,
        /// Estimated height, when the vehicle reports height.
        height_m: Option<f64>,
    },
    /// One truth sample, written five times a second. It lets a reader check
    /// the verdict against the path that the vehicle took.
    Truth {
        /// Seconds since the first message.
        at_s: f64,
        /// Truth position north of the launch point, in metres.
        north_m: f64,
        /// Truth position east of the launch point, in metres.
        east_m: f64,
        /// Truth height, when the vehicle reports height.
        height_m: Option<f64>,
        /// Truth heading in degrees true, when truth has attitude.
        heading_deg: Option<f64>,
        /// Armed state as the flight controller reports it, when it does.
        armed: Option<bool>,
        /// The executor phase at this sample.
        phase: Phase,
    },
    /// The host refused a control frame.
    FrameRejected {
        /// Seconds since the first message.
        at_s: f64,
        /// The host's reason code.
        reason: i32,
    },
    /// The verifier's report. It is the last line of a scenario run.
    Verdict {
        /// The report.
        report: &'a Report,
    },
    /// A live run ended. It has no verdict.
    LiveEnded {
        /// Seconds since the first message.
        at_s: f64,
    },
}

/// An append-only run record file.
pub(crate) struct RunRecord {
    path: PathBuf,
    file: File,
}

impl RunRecord {
    /// Creates the record file. An existing file is replaced.
    pub(crate) async fn create(path: &Path) -> Result<Self, PilotError> {
        let file = File::create(path)
            .await
            .map_err(|source| PilotError::Record {
                path: path.to_owned(),
                source,
            })?;
        Ok(Self {
            path: path.to_owned(),
            file,
        })
    }

    /// Appends one entry and flushes it, so a run that stops early keeps
    /// every line written before the stop.
    pub(crate) async fn append(&mut self, entry: &Entry<'_>) -> Result<(), PilotError> {
        let record = |source| PilotError::Record {
            path: self.path.clone(),
            source,
        };
        let mut line = serde_json::to_vec(entry).map_err(|source| record(source.into()))?;
        line.push(b'\n');
        self.file.write_all(&line).await.map_err(record)?;
        self.file.flush().await.map_err(record)
    }
}
