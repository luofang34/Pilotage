//! Refuses pipelines whose last reader can stop before its writer
//! does, in shell scripts that enable `pipefail`.
//!
//! `grep -q`, `grep -m`, `head` and `sed q` exit as soon as they have
//! their answer. A writer that still has output then gets `SIGPIPE`
//! and exits 141, and `pipefail` makes the whole pipeline false even
//! though the reader found its match. Whether the writer is still
//! writing depends on its buffer size and on scheduling, so the
//! script passes on one machine and fails at random on a loaded CI
//! runner. The safe forms read all input: `grep -F x >/dev/null` in
//! place of `grep -Fq x`, `sed -n 1p` in place of `head -1`, or a
//! here-string that tests a value captured once.

mod scan;

use std::fmt;
use std::path::Path;
use std::process::Command;

use crate::error::XtaskError;

#[cfg(test)]
mod tests;

/// One pipeline stage that can exit before its writer finishes.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Finding {
    pub(crate) path: String,
    pub(crate) line: usize,
    pub(crate) stage: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "FORBIDDEN: {}:{}: `| {}` can exit before its writer; under pipefail the writer's SIGPIPE fails the pipeline. Let the reader drain its input: drop the early exit and send the output to /dev/null, or test a captured value with a here-string.",
            self.path, self.line, self.stage
        )
    }
}

/// Scans every tracked shell script of the repository.
///
/// # Errors
///
/// Returns a typed [`XtaskError`] when git cannot list the tracked
/// scripts or a listed script cannot be read.
pub(crate) fn check(repo_root: &Path) -> Result<Vec<Finding>, XtaskError> {
    let mut findings = Vec::new();
    for path in tracked_shell_scripts(repo_root)? {
        let text =
            std::fs::read_to_string(repo_root.join(&path)).map_err(|source| XtaskError::Io {
                context: "reading a tracked shell script for the pipefail guard",
                source,
            })?;
        findings.extend(findings_in(&path, &text));
    }
    Ok(findings)
}

/// Names every early-exiting pipeline reader in one script. A script
/// without `pipefail` has none: there the reader's status alone
/// decides the pipeline.
pub(crate) fn findings_in(path: &str, text: &str) -> Vec<Finding> {
    if !enables_pipefail(text) {
        return Vec::new();
    }
    scan::readers(text)
        .into_iter()
        .filter(|reader| exits_early(&reader.words))
        .map(|reader| Finding {
            path: path.to_owned(),
            line: reader.line,
            stage: reader.words.join(" "),
        })
        .collect()
}

fn tracked_shell_scripts(repo_root: &Path) -> Result<Vec<String>, XtaskError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .args(["ls-files", "-z", "--", "*.sh"])
        .output()
        .map_err(|source| XtaskError::Io {
            context: "running git ls-files for the pipefail guard",
            source,
        })?;
    if !output.status.success() {
        return Err(XtaskError::Usage {
            message: format!(
                "git ls-files failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect())
}

/// True when a `set` command turns `pipefail` on, in any of its
/// spellings: `set -o pipefail`, `set -euo pipefail`, `set -e -o pipefail`.
fn enables_pipefail(text: &str) -> bool {
    text.lines().any(|line| {
        let mut words = line.split_whitespace();
        if words.next() != Some("set") {
            return false;
        }
        let mut option_named = false;
        for word in words {
            if word.starts_with('#') {
                return false;
            }
            if word == "pipefail" {
                return option_named;
            }
            option_named = word.starts_with('-') && word.contains('o');
        }
        false
    })
}

/// True when the reader named by `words` can exit before its input
/// ends. Leading `NAME=value` assignments are skipped.
pub(crate) fn exits_early(words: &[String]) -> bool {
    let mut words = words
        .iter()
        .skip_while(|word| is_assignment(word))
        .map(String::as_str);
    let Some(command) = words.next() else {
        return false;
    };
    let args: Vec<&str> = words.collect();
    match command.rsplit('/').next().unwrap_or(command) {
        "head" => true,
        "grep" | "egrep" | "fgrep" | "zgrep" => grep_stops_early(&args),
        "sed" => sed_quits(&args),
        _ => false,
    }
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty() && name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
    })
}

/// Short grep options that take the rest of their cluster, or the next
/// word, as their value.
const GREP_VALUE_OPTIONS: &str = "efABCdD";
const GREP_LONG_VALUE_OPTIONS: [&str; 9] = [
    "--regexp",
    "--file",
    "--after-context",
    "--before-context",
    "--context",
    "--label",
    "--include",
    "--exclude",
    "--exclude-dir",
];
const GREP_LONG_EARLY_EXITS: [&str; 5] = [
    "--quiet",
    "--silent",
    "--max-count",
    "--files-with-matches",
    "--files-without-match",
];

fn grep_stops_early(args: &[&str]) -> bool {
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if *arg == "--" {
            return false;
        }
        if arg.starts_with("--") {
            let long = *arg;
            let name = long.split('=').next().unwrap_or(long);
            if GREP_LONG_EARLY_EXITS.contains(&name) {
                return true;
            }
            if !long.contains('=') && GREP_LONG_VALUE_OPTIONS.contains(&name) {
                args.next();
            }
            continue;
        }
        let Some(cluster) = arg.strip_prefix('-') else {
            continue;
        };
        for (index, option) in cluster.char_indices() {
            if matches!(option, 'q' | 'l' | 'L' | 'm') {
                return true;
            }
            if GREP_VALUE_OPTIONS.contains(option) {
                if index + option.len_utf8() == cluster.len() {
                    args.next();
                }
                break;
            }
        }
    }
    false
}

/// True when a sed script holds a `q` or `Q` command. Without `-e`,
/// the first operand is the script.
fn sed_quits(args: &[&str]) -> bool {
    let mut scripts = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match *arg {
            "-e" | "--expression" => scripts.extend(args.next().copied()),
            "-f" | "--file" => {
                args.next();
            }
            option if option.starts_with('-') => {}
            operand => {
                if scripts.is_empty() {
                    scripts.push(operand);
                }
                break;
            }
        }
    }
    scripts.iter().any(|script| script_quits(script))
}

fn script_quits(script: &str) -> bool {
    let chars: Vec<char> = script.chars().collect();
    chars.iter().enumerate().any(|(index, c)| {
        if !matches!(c, 'q' | 'Q') {
            return false;
        }
        let before = index
            .checked_sub(1)
            .and_then(|previous| chars.get(previous))
            .copied();
        let command_start = before.is_none_or(|b| {
            matches!(b, ';' | '{' | '}' | '$' | '/') || b.is_ascii_digit() || b.is_whitespace()
        });
        let after = chars
            .iter()
            .skip(index + 1)
            .find(|next| !next.is_ascii_digit() && !next.is_whitespace());
        command_start && after.is_none_or(|a| matches!(a, ';' | '}'))
    })
}
