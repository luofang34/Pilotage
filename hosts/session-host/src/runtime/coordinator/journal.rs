//! The coordinator's handover journal (ADR-0052): the handovers as one
//! protobuf message, written to a temporary file, synced and renamed over the
//! journal before the coordinator sends work to a member, so a restart reads
//! back every handover it had committed.

use std::path::{Path, PathBuf};

use pilotage_protocol::wire;
use prost::Message as _;

use crate::error::HostError;

#[cfg(test)]
mod tests;

/// The handovers in the journal at `path`; none when it does not exist.
///
/// # Errors
/// [`HostError::CoordinatorJournal`] for a journal that cannot be read or
/// decoded.
pub(crate) fn read_blocking(path: &Path) -> Result<Vec<wire::MissionHandover>, HostError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(journal_error(path, &error.to_string())),
    };
    wire::MissionHandoverJournal::decode(bytes.as_slice())
        .map(|journal| journal.handovers)
        .map_err(|error| journal_error(path, &error.to_string()))
}

/// Writes `handovers` to the journal at `path`, durably.
///
/// # Errors
/// The I/O error of the write, the sync or the rename.
pub(crate) async fn write(path: &Path, handovers: &[wire::MissionHandover]) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt as _;
    let bytes = wire::MissionHandoverJournal {
        handovers: handovers.to_vec(),
    }
    .encode_to_vec();
    let mut temporary = PathBuf::from(path);
    temporary.as_mut_os_string().push(".tmp");
    let mut file = tokio::fs::File::create(&temporary).await?;
    file.write_all(&bytes).await?;
    file.sync_all().await?;
    drop(file);
    tokio::fs::rename(&temporary, path).await
}

fn journal_error(path: &Path, reason: &str) -> HostError {
    HostError::CoordinatorJournal {
        path: path.to_path_buf(),
        reason: reason.to_owned(),
    }
}
