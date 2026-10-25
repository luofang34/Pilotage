//! The lane journal (ADR-0052): the facts of the lane's run as one protobuf
//! message. A write goes to a temporary file, which is synced and renamed
//! over the journal, and then the directory is synced, so a restart reads
//! back the run as the last write left it.
//!
//! The host starts no run before the journal holds it, and a run of
//! operations begins its next operation only once the journal holds how
//! the ones before it ended. So while a write fails, no new work begins;
//! a hold, an abort and an answer still pass, as they need no record.

use std::path::{Path, PathBuf};

use pilotage_protocol::wire;
use prost::Message as _;
use tokio::sync::watch;
use tracing::{error, info};

use crate::error::HostError;

/// The run in the journal at `path`; none when the journal does not exist
/// or holds no run.
///
/// # Errors
/// [`HostError::LaneJournal`] for a journal that cannot be read or decoded.
pub(crate) async fn read(path: &Path) -> Result<Option<wire::MissionLaneRecord>, HostError> {
    let bytes = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(journal_error(path, &error.to_string())),
    };
    let record = wire::MissionLaneRecord::decode(bytes.as_slice())
        .map_err(|error| journal_error(path, &error.to_string()))?;
    Ok(record.proposal.is_some().then_some(record))
}

/// Writes `record`, or a record of no run, to the journal at `path`,
/// durably.
async fn write(path: &Path, record: Option<&wire::MissionLaneRecord>) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt as _;
    let bytes = record.cloned().unwrap_or_default().encode_to_vec();
    let mut temporary = PathBuf::from(path);
    temporary.as_mut_os_string().push(".tmp");
    let mut file = tokio::fs::File::create(&temporary).await?;
    file.write_all(&bytes).await?;
    file.sync_all().await?;
    drop(file);
    tokio::fs::rename(&temporary, path).await?;
    // The rename is durable only once the directory is.
    let directory = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    tokio::fs::File::open(directory).await?.sync_all().await
}

pub(super) fn journal_error(path: &Path, reason: &str) -> HostError {
    HostError::LaneJournal {
        path: path.to_path_buf(),
        reason: reason.to_owned(),
    }
}

/// The journal of one lane, and whether it holds the lane's run now.
#[derive(Debug)]
pub(crate) struct LaneJournal {
    /// The journal file; none when the host keeps no journal.
    path: Option<PathBuf>,
    /// The record the journal holds, as last read or written.
    held: Option<wire::MissionLaneRecord>,
    /// The last write failed.
    failing: bool,
    /// The tasks of the run whose end the journal holds.
    ended: watch::Sender<usize>,
}

impl LaneJournal {
    /// The journal at `path`, which holds `held`.
    pub(crate) fn new(path: Option<PathBuf>, held: Option<wire::MissionLaneRecord>) -> Self {
        let ended = watch::channel(ended_tasks(held.as_ref())).0;
        Self {
            path,
            held,
            failing: false,
            ended,
        }
    }

    /// How many tasks of the run the journal holds as ended, as it changes.
    pub(crate) fn ended(&self) -> watch::Receiver<usize> {
        self.ended.subscribe()
    }

    /// Makes the journal hold `record`. Returns whether it does.
    pub(crate) async fn keep(&mut self, record: Option<wire::MissionLaneRecord>) -> bool {
        if let Some(path) = self.path.as_ref()
            && record != self.held
        {
            if let Err(failure) = write(path, record.as_ref()).await {
                if !self.failing {
                    error!(journal = %path.display(), %failure, "the lane journal cannot be written; no new work begins until it can");
                }
                self.failing = true;
                return false;
            }
            if self.failing {
                info!(journal = %path.display(), "the lane journal is written again");
            }
            self.failing = false;
        }
        self.ended.send_replace(ended_tasks(record.as_ref()));
        self.held = record;
        true
    }
}

/// The tasks of the run in `record` whose execution ended.
fn ended_tasks(record: Option<&wire::MissionLaneRecord>) -> usize {
    use wire::MissionTaskExecution as Execution;
    record.map_or(0, |record| {
        record
            .tasks
            .iter()
            .filter(|task| {
                matches!(
                    Execution::try_from(task.execution),
                    Ok(Execution::Completed | Execution::Cancelled | Execution::Failed)
                )
            })
            .count()
    })
}

#[cfg(test)]
mod tests;
