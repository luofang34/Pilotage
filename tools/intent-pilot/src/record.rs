//! The run record: one JSON object on each line, in event order.
//!
//! The record keeps the classifier's answer next to the intent the
//! scenario author wrote, so a reader sees each parse graded. It keeps
//! the verifier's report as the last line.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use crate::classifier::Reading;
use crate::error::PilotError;
use crate::scenario::{Intent, Scenario};
use crate::sequencer::Phase;
use crate::verdict::Report;

/// One line of the run record.
#[derive(Debug, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub(crate) enum Entry<'a> {
    /// The scenario and the identities of the two ends.
    Start {
        /// The scenario document, complete.
        scenario: &'a Scenario,
        /// The model name the classifier reported.
        classifier_model: &'a str,
        /// The legend text the classifier receives.
        legend: &'a str,
        /// The control-source identity announced to the host.
        profile_id: &'a str,
    },
    /// An operator message and how the classifier read it.
    Message {
        /// Seconds since the first message.
        at_s: f64,
        /// The classifier's reading.
        reading: &'a Reading,
        /// The intent the scenario author wrote for this message.
        means: &'a Intent,
        /// True when the reading equals the author's intent.
        parse_correct: bool,
    },
    /// The classifier failed on a message. The flight keeps its last
    /// destination.
    ClassifierFault {
        /// Seconds since the first message.
        at_s: f64,
        /// The failure, in words.
        detail: String,
    },
    /// The sequencer changed phase.
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
    /// One simulator-truth sample, written five times a second. It lets a reader
    /// check the verdict against the path the vehicle took.
    Truth {
        /// Seconds since the first message.
        at_s: f64,
        /// Truth position north of the launch point, in metres.
        north_m: f64,
        /// Truth position east of the launch point, in metres.
        east_m: f64,
        /// Truth height, when the vehicle reports height.
        height_m: Option<f64>,
        /// Armed state as the flight controller reports it, when it does.
        armed: Option<bool>,
        /// The sequencer phase at this sample.
        phase: Phase,
    },
    /// The host refused a control frame.
    FrameRejected {
        /// Seconds since the first message.
        at_s: f64,
        /// The host's reason code.
        reason: i32,
    },
    /// The verifier's report. It is the last line.
    Verdict {
        /// The report.
        report: &'a Report,
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
