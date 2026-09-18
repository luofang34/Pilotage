//! The append-only run ledger.
//!
//! A held-out suite gives an honest number one time. The ledger does not stop
//! a second run. It counts the runs, so a number from a second run cannot pass
//! as a number from a first run.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use pilotage_agent::{AdapterDeclaration, Suite, SuiteReport};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::error::EvalError;

/// One run of one adapter on one suite digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Entry {
    /// The suite identifier.
    pub suite_id: String,
    /// SHA-256 of the suite file.
    pub suite_sha256: String,
    /// True when the suite is a held-out suite.
    pub held_out: bool,
    /// The adapter name and version.
    pub adapter: String,
    /// The model identity.
    pub model: String,
    /// The run time in seconds since the Unix epoch.
    pub at_unix_s: u64,
    /// Correct cases.
    pub correct: usize,
    /// All cases.
    pub total: usize,
}

impl Entry {
    pub(crate) fn new(
        suite: &Suite,
        suite_sha256: &str,
        declaration: &AdapterDeclaration,
        summary: &SuiteReport,
    ) -> Self {
        Self {
            suite_id: suite.id.clone(),
            suite_sha256: suite_sha256.to_owned(),
            held_out: suite.held_out,
            adapter: declaration.adapter.clone(),
            model: declaration.model.clone(),
            at_unix_s: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |since| since.as_secs()),
            correct: summary.cases.correct,
            total: summary.cases.total,
        }
    }

    fn same_run_key(&self, other: &Self) -> bool {
        self.suite_sha256 == other.suite_sha256
            && self.adapter == other.adapter
            && self.model == other.model
    }
}

/// Appends `entry` and returns its run number: one for the first run of this
/// adapter and model on this suite digest.
pub(crate) async fn append(path: &Path, entry: Entry) -> Result<usize, EvalError> {
    let earlier = match tokio::fs::read_to_string(path).await {
        Ok(text) => text
            .lines()
            .filter_map(|line| serde_json::from_str::<Entry>(line).ok())
            .filter(|earlier| earlier.same_run_key(&entry))
            .count(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(source) => {
            return Err(EvalError::Read {
                path: path.to_owned(),
                source,
            });
        }
    };
    let write = |source| EvalError::Write {
        path: path.to_owned(),
        source,
    };
    let mut line = serde_json::to_vec(&entry).map_err(|source| write(source.into()))?;
    line.push(b'\n');
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .map_err(write)?;
    file.write_all(&line).await.map_err(write)?;
    file.flush().await.map_err(write)?;
    Ok(earlier.wrapping_add(1))
}
