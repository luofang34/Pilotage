//! The handover journal a fleet's leads write next to their member lists.

use std::path::Path;

use crate::error::XtaskError;

/// Removes the handover journal of the lead whose member list is `members`:
/// a new fleet's aircraft fly none of an earlier fleet's work.
///
/// # Errors
/// [`XtaskError::Io`] when the journal exists and cannot be removed.
pub(super) fn forget_blocking(members: &Path) -> Result<(), XtaskError> {
    let mut journal = members.as_os_str().to_owned();
    journal.push(".handovers");
    match std::fs::remove_file(&journal) {
        Err(source) if source.kind() != std::io::ErrorKind::NotFound => Err(XtaskError::Io {
            context: "removing an earlier fleet's handover journal",
            source,
        }),
        _ => Ok(()),
    }
}
