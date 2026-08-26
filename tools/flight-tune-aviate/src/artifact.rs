use std::path::Path;

use crate::AviateSupervisorError;

mod removal;
mod root;
mod staging;

pub(crate) use removal::{remove_artifact_root, remove_staged, stabilize_absent_artifact_root};
pub(crate) use root::{create_artifact_root, inspect_directory, validate_directory};
pub(crate) use staging::{inspect_staged, stage_executable};

pub(crate) const SUPERVISOR_ARTIFACT: &str = "supervisor";
pub(crate) const TARGET_ARTIFACT: &str = "target";

pub(super) fn io_error(
    operation: &'static str,
    path: &Path,
    source: std::io::Error,
) -> AviateSupervisorError {
    AviateSupervisorError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}
