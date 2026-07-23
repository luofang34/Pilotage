//! Preserves the previous sim run's stage logs before a new launch
//! overwrites them.
//!
//! Stage logs are the primary incident record (writer deadlines, budget
//! transitions, link-loss policy); truncating them at the next launch
//! destroys exactly the evidence an operator relaunches to investigate.
//! Each launch moves the prior run's `*.log` files into `prev/run-<epoch>/`
//! keyed by the newest log's modification time, and prunes the oldest
//! archives beyond a bounded count.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::error::XtaskError;

/// Archived runs kept under `prev/`; the oldest beyond this are removed.
const MAX_ARCHIVED_RUNS: usize = 10;

/// Moves any `*.log` files directly inside `log_dir` into
/// `prev/run-<epoch-seconds>/` and prunes old archives. Returns the archive
/// directory, or `None` when there was nothing to preserve.
///
/// # Errors
///
/// Returns [`XtaskError::Io`] when the log directory cannot be scanned or a
/// log file cannot be moved into the archive.
pub fn archive_previous_logs(log_dir: &Path) -> Result<Option<PathBuf>, XtaskError> {
    let logs = previous_logs(log_dir)?;
    let Some(stamp) = newest_mtime_epoch(&logs) else {
        return Ok(None);
    };
    let archive = log_dir.join("prev").join(format!("run-{stamp}"));
    std::fs::create_dir_all(&archive).map_err(|source| XtaskError::Io {
        context: "creating the previous-run log archive",
        source,
    })?;
    for log in &logs {
        let Some(name) = log.file_name() else {
            continue;
        };
        std::fs::rename(log, archive.join(name)).map_err(|source| XtaskError::Io {
            context: "moving a previous-run log into the archive",
            source,
        })?;
    }
    prune_old_archives(&log_dir.join("prev"))?;
    Ok(Some(archive))
}

fn previous_logs(log_dir: &Path) -> Result<Vec<PathBuf>, XtaskError> {
    let entries = match std::fs::read_dir(log_dir) {
        Ok(entries) => entries,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => {
            return Err(XtaskError::Io {
                context: "scanning the session log directory",
                source,
            });
        }
    };
    let mut logs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "log"))
        .collect();
    logs.sort();
    Ok(logs)
}

fn newest_mtime_epoch(logs: &[PathBuf]) -> Option<u64> {
    logs.iter()
        .filter_map(|log| log.metadata().ok()?.modified().ok())
        .filter_map(|mtime| mtime.duration_since(UNIX_EPOCH).ok())
        .map(|since| since.as_secs())
        .max()
}

fn prune_old_archives(prev_dir: &Path) -> Result<(), XtaskError> {
    let Ok(entries) = std::fs::read_dir(prev_dir) else {
        return Ok(());
    };
    let mut runs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    // `run-<epoch>` names sort chronologically while epoch seconds keep the
    // same digit count; equal-width numeric names make lexical order safe.
    runs.sort();
    while runs.len() > MAX_ARCHIVED_RUNS {
        let oldest = runs.remove(0);
        std::fs::remove_dir_all(&oldest).map_err(|source| XtaskError::Io {
            context: "pruning the oldest archived run logs",
            source,
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::panic)]
    use std::path::PathBuf;

    use super::{MAX_ARCHIVED_RUNS, archive_previous_logs};

    /// Fresh scratch directory, removed on drop; keeps the tests std-only.
    struct ScratchDir(PathBuf);

    impl ScratchDir {
        fn new(label: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("xtask-log-archive-{label}-{}", std::process::id()));
            std::fs::remove_dir_all(&dir).ok();
            std::fs::create_dir_all(&dir).expect("scratch dir");
            Self(dir)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    #[test]
    fn logs_move_into_a_run_archive_and_the_log_dir_is_left_clean() {
        let dir = ScratchDir::new("move");
        std::fs::write(dir.path().join("session-host.log"), b"evidence").expect("write");
        std::fs::write(dir.path().join("gazebo.log"), b"gz").expect("write");
        std::fs::write(dir.path().join("supervisor.pid"), b"123").expect("write");

        let archive = archive_previous_logs(dir.path())
            .expect("archive succeeds")
            .expect("logs existed");
        let preserved =
            std::fs::read_to_string(archive.join("session-host.log")).expect("preserved");
        assert_eq!(preserved, "evidence", "log content survives the move");
        assert!(
            !dir.path().join("session-host.log").exists(),
            "the live log dir no longer holds the previous run's log"
        );
        assert!(
            dir.path().join("supervisor.pid").exists(),
            "non-log files stay untouched"
        );
    }

    #[test]
    fn an_empty_log_dir_archives_nothing() {
        let dir = ScratchDir::new("empty");
        assert!(
            archive_previous_logs(dir.path())
                .expect("archive succeeds")
                .is_none()
        );
        assert!(!dir.path().join("prev").exists(), "no archive dir appears");
    }

    #[test]
    fn archives_beyond_the_bound_prune_oldest_first() {
        let dir = ScratchDir::new("prune");
        let prev = dir.path().join("prev");
        for stamp in 0..=MAX_ARCHIVED_RUNS {
            std::fs::create_dir_all(prev.join(format!("run-{stamp:010}"))).expect("mkdir");
        }
        std::fs::write(dir.path().join("viewer.log"), b"v").expect("write");
        archive_previous_logs(dir.path()).expect("archive succeeds");

        assert!(
            !prev.join(format!("run-{:010}", 0)).exists(),
            "the oldest archive is pruned"
        );
        let kept = std::fs::read_dir(&prev).expect("read prev").count();
        assert_eq!(kept, MAX_ARCHIVED_RUNS, "the bound holds after pruning");
    }
}
