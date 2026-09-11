use pilotage_data_packages::{InstalledRelease, Selection};

#[derive(serde::Serialize)]
pub(super) struct DataSnapshot {
    pub installed: Vec<InstalledRelease>,
    pub selections: Vec<Selection>,
}

/// Requirements for a retained data selection.
#[derive(Debug, uniffi::Record)]
pub struct DataSelectionRequest {
    /// Selection owner, such as `active-terrain`.
    pub name: String,
    /// Exact installed release ID.
    pub release_id: String,
    /// Prevent changes until the owner releases this selection.
    pub pinned: bool,
    /// Evaluation time in Unix seconds.
    pub now: i64,
    /// Permit the development publication channel.
    pub development: bool,
    /// Permit explicit inspection outside the validity interval.
    pub allow_outside_validity: bool,
    /// Features supplied by the renderer and data consumers.
    pub renderer_capabilities: Vec<String>,
}

/// Received bytes for the object that is being downloaded.
#[derive(Debug, uniffi::Record)]
pub struct DataTransferProgress {
    /// Artifact path within its release.
    pub path: String,
    /// Bytes retained in persistent staging storage.
    pub received: u64,
    /// Required artifact length.
    pub total: u64,
}

/// Transfer events delivered on the data worker.
#[uniffi::export(callback_interface)]
pub trait DataDownloadObserver: Send + Sync {
    /// Report a completed transfer chunk.
    fn progress(&self, progress: DataTransferProgress);
}
