//! Content stamps for gitignored build artifacts: a build is skipped only
//! when the artifact exists AND the recorded stamp — the working-tree
//! content hashes of every source input plus the toolchain versions —
//! matches the current one. An existence-only check would happily serve a
//! stale artifact after a source or toolchain change; the stamp makes
//! "fresh checkout", "edited source", and "upgraded toolchain" all rebuild.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Whether a stamped artifact can be reused: it must exist and its stored
/// stamp must equal the freshly computed one. A missing or unreadable
/// stored stamp rebuilds (fail closed toward rebuilding, never toward
/// serving something stale).
pub(crate) fn artifact_is_fresh(
    artifacts_exist: bool,
    stored: Option<&str>,
    current: Option<&str>,
) -> bool {
    match (artifacts_exist, stored, current) {
        (true, Some(stored), Some(current)) => stored == current,
        _ => false,
    }
}

/// Computes the content stamp: one line per toolchain probe output, then one
/// line per tracked source file with its WORKING-TREE content hash (so an
/// unstaged edit changes the stamp). Returns `None` when git is unavailable
/// — the caller falls back to rebuilding.
pub(crate) fn source_stamp(
    repo_root: &Path,
    source_paths: &[&str],
    toolchain_probes: &[&[&str]],
) -> Option<String> {
    let mut stamp = String::new();
    for probe in toolchain_probes {
        let (program, args) = probe.split_first()?;
        let output = Command::new(program)
            .args(args)
            .current_dir(repo_root)
            .output();
        let line = match output {
            Ok(output) if output.status.success() => {
                String::from_utf8_lossy(&output.stdout).trim().to_owned()
            }
            // A missing probe is itself part of the stamp: installing the
            // tool later must rebuild.
            _ => format!("{program}: unavailable"),
        };
        stamp.push_str(&line);
        stamp.push('\n');
    }
    let files = tracked_files(repo_root, source_paths)?;
    let hashes = hash_files(repo_root, &files)?;
    for (file, hash) in files.iter().zip(hashes) {
        stamp.push_str(&format!("{hash}  {}\n", file.display()));
    }
    Some(stamp)
}

/// The tracked files under `source_paths`, in git's stable listing order.
fn tracked_files(repo_root: &Path, source_paths: &[&str]) -> Option<Vec<PathBuf>> {
    let output = Command::new("git")
        .args(["ls-files", "-z", "--"])
        .args(source_paths)
        .current_dir(repo_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| PathBuf::from(String::from_utf8_lossy(path).into_owned()))
            .collect(),
    )
}

/// Working-tree content hashes via `git hash-object --stdin-paths`, one per
/// input line, so the stamp reflects what is ON DISK rather than what is
/// staged or committed.
fn hash_files(repo_root: &Path, files: &[PathBuf]) -> Option<Vec<String>> {
    use std::io::Write;
    let mut child = Command::new("git")
        .args(["hash-object", "--stdin-paths"])
        .current_dir(repo_root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    {
        let stdin = child.stdin.as_mut()?;
        for file in files {
            writeln!(stdin, "{}", file.display()).ok()?;
        }
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    let hashes: Vec<String> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect();
    (hashes.len() == files.len()).then_some(hashes)
}

/// Reads a stored stamp, `None` when absent or unreadable.
pub(crate) fn read_stamp(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Records a stamp after a successful build; best-effort (a write failure
/// only costs a redundant rebuild next run, never a stale artifact).
pub(crate) fn write_stamp(path: &Path, stamp: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(path, stamp).ok();
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests {
    use super::artifact_is_fresh;

    #[test]
    fn freshness_requires_existing_artifacts_and_a_matching_stamp() {
        assert!(artifact_is_fresh(true, Some("a\n"), Some("a\n")));
        assert!(
            !artifact_is_fresh(true, Some("a\n"), Some("b\n")),
            "a changed source or toolchain rebuilds"
        );
        assert!(
            !artifact_is_fresh(false, Some("a\n"), Some("a\n")),
            "a missing artifact rebuilds regardless of the stamp"
        );
        assert!(
            !artifact_is_fresh(true, None, Some("a\n")),
            "no recorded stamp rebuilds"
        );
        assert!(
            !artifact_is_fresh(true, Some("a\n"), None),
            "an uncomputable stamp rebuilds"
        );
    }
}
