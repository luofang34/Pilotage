//! The embeddable host core reads no clock: every time in it comes from
//! the host that steps it, so the same inputs give the same bytes and a
//! browser or a simulator can run it at any rate.
//!
//! The guard scans the core crate and the crates its embedded path links
//! (the Aviate adapter, the MAVLink link cache, the uplink policy) for
//! `Instant::now`, `SystemTime::now`, `tokio::time` and an `.elapsed()`
//! that is not the uplink's own clock-based `self.elapsed()`. Test files
//! are not scanned. A file of a dependency crate that only a process host
//! links (behind `net` or `sim`) is listed with the number of reads it
//! holds and why; a new read in it, or any read in a file not listed,
//! fails the guard.

use std::path::{Path, PathBuf};

use crate::error::XtaskError;

#[cfg(test)]
mod tests;

/// The guard's name in the guard report.
pub(crate) const NAME: &str = "embedded-clock";

/// The crate directories the embedded path links, relative to the
/// repository root.
const SCANNED: &[&str] = &[
    "crates/pilotage-host-core/src",
    "adapters/aviate/src",
    "crates/pilotage-mavlink/src",
    "crates/pilotage-flight-uplink/src",
];

/// One file that holds clock reads only a process host reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Allowed {
    /// The file, relative to the repository root.
    pub(crate) path: &'static str,
    /// The number of reads the file holds.
    pub(crate) reads: usize,
    /// Why no embedded host reaches them.
    pub(crate) reason: &'static str,
}

/// Every file with reads an embedded host does not reach.
pub(crate) const ALLOWED: &[Allowed] = &[
    Allowed {
        path: "adapters/aviate/src/adapter.rs",
        reads: 1,
        reason: "test-only constructor",
    },
    Allowed {
        path: "adapters/aviate/src/adapter/camera.rs",
        reads: 1,
        reason: "sim camera bridge",
    },
    Allowed {
        path: "adapters/aviate/src/adapter/pointing.rs",
        reads: 4,
        reason: "sim payload view",
    },
    Allowed {
        path: "adapters/aviate/src/adapter/shm_sampling.rs",
        reads: 1,
        reason: "sim truth oracle",
    },
    Allowed {
        path: "adapters/aviate/src/adapter/startup/process.rs",
        reads: 1,
        reason: "net start",
    },
    Allowed {
        path: "adapters/aviate/src/clock.rs",
        reads: 1,
        reason: "the net System clock",
    },
    Allowed {
        path: "adapters/aviate/src/shm.rs",
        reads: 2,
        reason: "sim shared memory",
    },
    Allowed {
        path: "adapters/aviate/src/uplink.rs",
        reads: 1,
        reason: "net UDP constructor",
    },
    Allowed {
        path: "crates/pilotage-mavlink/src/camera_endpoint.rs",
        reads: 3,
        reason: "net",
    },
    Allowed {
        path: "crates/pilotage-mavlink/src/camera_endpoint/execution.rs",
        reads: 1,
        reason: "net",
    },
    Allowed {
        path: "crates/pilotage-mavlink/src/camera_endpoint/observation.rs",
        reads: 1,
        reason: "net",
    },
    Allowed {
        path: "crates/pilotage-mavlink/src/gimbal_control.rs",
        reads: 3,
        reason: "net",
    },
    Allowed {
        path: "crates/pilotage-mavlink/src/gimbal_control/discovery.rs",
        reads: 1,
        reason: "net",
    },
    Allowed {
        path: "crates/pilotage-mavlink/src/gimbal_control/release.rs",
        reads: 6,
        reason: "net",
    },
    Allowed {
        path: "crates/pilotage-mavlink/src/gimbal_control/transport.rs",
        reads: 3,
        reason: "net",
    },
    Allowed {
        path: "crates/pilotage-mavlink/src/gimbal_endpoint/observation.rs",
        reads: 1,
        reason: "net",
    },
    Allowed {
        path: "crates/pilotage-mavlink/src/link/apply.rs",
        reads: 1,
        reason: "net apply_messages",
    },
    Allowed {
        path: "crates/pilotage-mavlink/src/link/defaults.rs",
        reads: 1,
        reason: "Default, for process hosts",
    },
    Allowed {
        path: "crates/pilotage-mavlink/src/link/fence.rs",
        reads: 2,
        reason: "net send fences",
    },
    Allowed {
        path: "crates/pilotage-mavlink/src/link/fence/flight.rs",
        reads: 1,
        reason: "net send fences",
    },
    Allowed {
        path: "crates/pilotage-mavlink/src/link/outbound/lanes.rs",
        reads: 4,
        reason: "net",
    },
    Allowed {
        path: "crates/pilotage-mavlink/src/link/task.rs",
        reads: 4,
        reason: "net",
    },
    Allowed {
        path: "crates/pilotage-mavlink/src/link/udp.rs",
        reads: 1,
        reason: "net",
    },
    Allowed {
        path: "crates/pilotage-mavlink/src/payload_execution.rs",
        reads: 4,
        reason: "net payload writes",
    },
    Allowed {
        path: "crates/pilotage-mavlink/src/payload_execution/write.rs",
        reads: 3,
        reason: "net",
    },
];

/// Every refused clock read, and every listed file whose count changed.
///
/// # Errors
///
/// [`XtaskError::Io`] when a source tree cannot be read.
pub(crate) fn check(repo_root: &Path) -> Result<Vec<String>, XtaskError> {
    let mut refusals = Vec::new();
    for dir in SCANNED {
        let mut files = Vec::new();
        rust_files(&repo_root.join(dir), &mut files)?;
        files.sort();
        for file in files {
            let relative = file.strip_prefix(repo_root).map_or_else(
                |_| file.display().to_string(),
                |path| path.display().to_string(),
            );
            if is_test_file(&relative) {
                continue;
            }
            let source = std::fs::read_to_string(&file).map_err(|source| XtaskError::Io {
                context: "reading a source file for the embedded-clock guard",
                source,
            })?;
            refusals.extend(judge(&relative, &reads(&source)));
        }
    }
    Ok(refusals)
}

/// The refusals for one file's reads: none for a listed file whose count
/// matches, one line per read otherwise.
pub(crate) fn judge(path: &str, found: &[(usize, String)]) -> Vec<String> {
    if found.is_empty() {
        return Vec::new();
    }
    if let Some(allowed) = ALLOWED.iter().find(|allowed| allowed.path == path) {
        if allowed.reads == found.len() {
            return Vec::new();
        }
        return vec![format!(
            "FORBIDDEN: {path} holds {} clock reads, {} listed ({})",
            found.len(),
            allowed.reads,
            allowed.reason
        )];
    }
    found
        .iter()
        .map(|(line, text)| {
            format!("FORBIDDEN: a clock read on the embedded path: {path}:{line}: {text}")
        })
        .collect()
}

/// The lines of `source` that read a clock, with their numbers; comments
/// are not code.
pub(crate) fn reads(source: &str) -> Vec<(usize, String)> {
    source
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let code = line.split("//").next().unwrap_or_default();
            reads_clock(code).then(|| (index + 1, line.trim().to_owned()))
        })
        .collect()
}

fn reads_clock(code: &str) -> bool {
    code.contains("Instant::now(")
        || code.contains("SystemTime::now(")
        || code.contains("tokio::time")
        || code.replace("self.elapsed()", "").contains(".elapsed()")
}

fn is_test_file(path: &str) -> bool {
    let name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    name == "tests.rs"
        || name.ends_with("_tests.rs")
        || name == "test_support.rs"
        || path.split('/').any(|part| part == "tests")
}

fn rust_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), XtaskError> {
    let entries = std::fs::read_dir(dir).map_err(|source| XtaskError::Io {
        context: "reading a source directory for the embedded-clock guard",
        source,
    })?;
    for entry in entries {
        let path = entry
            .map_err(|source| XtaskError::Io {
                context: "reading a source directory entry for the embedded-clock guard",
                source,
            })?
            .path();
        if path.is_dir() {
            rust_files(&path, files)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
    Ok(())
}
