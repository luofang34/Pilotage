//! Keeps the Aviate adapter's flight-response constants in the typed
//! control-feel artifact.
//!
//! A shaping constant written in the adapter source flies a law nobody
//! reviewed: it bypasses the profile the operator chose and the digest
//! that names it. So every module-level `const` or `static` in
//! `adapters/aviate/src` whose name or type reads as a response value is
//! refused unless it is a listed exemption, and each exemption may be
//! used once. The uplink also must not expose a public profile mutation,
//! which would let a caller install a law past the execution target.

use std::path::{Path, PathBuf};

use crate::error::XtaskError;

#[cfg(test)]
mod tests;

/// The source tree the guard scans, relative to the repository root.
pub(crate) const AVIATE_SRC: &str = "adapters/aviate/src";

/// Name tokens that read as a flight-response value.
const RESPONSE_TOKENS: &[&str] = &[
    "HORIZONTAL",
    "VERTICAL",
    "YAW",
    "TILT",
    "THRUST",
    "THROTTLE",
    "TAKEOFF",
    "ACCEL",
    "JERK",
    "DEADZONE",
    "EXPO",
    "HOLD",
    "RATE",
    "ROLL",
    "PITCH",
    "SPEED",
    "RESPONSE",
    "CURVE",
    "NEUTRAL",
    "DWELL",
    "RELEASE",
    "REVERSAL",
    "RAMP",
    "SLEW",
    "QUIET",
    "LIMIT",
    "APPLY",
    "SETTLE",
    "DT",
    "TIME",
    "TIMEOUT",
    "TICKS",
    "STEPS",
    "PERIOD",
    "DELAY",
    "THRESHOLD",
];

/// Types that read as a flight-response value.
const RESPONSE_TYPES: &[&str] = &["f32", "f64", "Duration"];

/// Constants that are not flight-response values, each with its reason.
pub(crate) const EXEMPTIONS: &[(&str, &str, &str)] = &[
    (
        "link/measurement.rs",
        "RESET_RECEIVE_DWELL",
        "estimator reset detection",
    ),
    (
        "link/measurement.rs",
        "RESET_CANDIDATE_MAX_MS",
        "estimator reset detection",
    ),
    (
        "link/measurement.rs",
        "RESET_SOURCE_DWELL_MS",
        "estimator reset detection",
    ),
    (
        "link/measurement.rs",
        "RESET_SILENCE",
        "estimator reset detection",
    ),
    (
        "uplink.rs",
        "MAX_DT_S",
        "stall bound on the yaw-rate integration step",
    ),
    (
        "uplink.rs",
        "ARM_QUIET",
        "the FC's post-arm command quiet interval",
    ),
    (
        "uplink.rs",
        "HOLD_KEEPALIVE",
        "repeats a latched link-loss hold inside the FC's 500 ms command timeout",
    ),
    (
        "adapter/control.rs",
        "RESET_CLEAR_DEADBAND",
        "reset-latch neutral test",
    ),
    ("adapter.rs", "GIMBAL_NEUTRAL_BUTTON", "a button id"),
    ("adapter.rs", "ROLL_AXIS", "an axis id"),
    ("adapter.rs", "PITCH_AXIS", "an axis id"),
    ("adapter.rs", "THROTTLE_AXIS", "an axis id"),
    ("adapter.rs", "YAW_AXIS", "an axis id"),
    ("adapter.rs", "WITHHOLD_AFTER", "telemetry staleness"),
    (
        "adapter/shm_sampling.rs",
        "REATTACH_INTERVAL",
        "shared-memory reattach",
    ),
    ("adapter/pointing.rs", "PAN_LIMIT_RAD", "gimbal pointing"),
    ("adapter/pointing.rs", "TILT_MIN_RAD", "gimbal pointing"),
    ("adapter/pointing.rs", "TILT_MAX_RAD", "gimbal pointing"),
    (
        "adapter/pointing.rs",
        "MAX_PITCH_RATE_RPS",
        "gimbal pointing",
    ),
    ("adapter/pointing.rs", "MAX_YAW_RATE_RPS", "gimbal pointing"),
    (
        "adapter/pointing.rs",
        "INTEGRATION_STEP_S",
        "gimbal pointing",
    ),
    (
        "adapter/pointing.rs",
        "PAYLOAD_VIEW_HOLD",
        "gimbal camera view",
    ),
];

/// One refused declaration, or the public profile entry.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Finding {
    /// Path relative to [`AVIATE_SRC`].
    pub(crate) path: String,
    /// The refused identifier, or `custom-profile-entry`.
    pub(crate) name: String,
}

/// Every finding under `repo_root`.
///
/// # Errors
///
/// Returns [`XtaskError::Io`] when the Aviate tree cannot be read; a
/// missing tree is an error, since the guard would then check nothing.
pub(crate) fn check(repo_root: &Path) -> Result<Vec<Finding>, XtaskError> {
    let root = repo_root.join(AVIATE_SRC);
    let mut files = Vec::new();
    collect(&root, &mut files)?;
    files.sort();
    let mut findings = Vec::new();
    let mut used: Vec<(String, String)> = Vec::new();
    for file in files {
        let relative = file
            .strip_prefix(&root)
            .map_or_else(|_| file.display().to_string(), |p| p.display().to_string());
        let text = std::fs::read_to_string(&file).map_err(|source| XtaskError::Io {
            context: "reading an Aviate adapter source for the control-feel guard",
            source,
        })?;
        if relative == "uplink.rs" && text.lines().any(exposes_profile_entry) {
            findings.push(Finding {
                path: relative.clone(),
                name: "custom-profile-entry".to_owned(),
            });
        }
        for line in text.lines() {
            let Some((name, type_text)) = declaration(line) else {
                continue;
            };
            if !reads_as_response(name, type_text) {
                continue;
            }
            let key = (relative.clone(), name.to_owned());
            let exempt = EXEMPTIONS
                .iter()
                .any(|(path, exempt, _)| *path == relative && *exempt == name);
            if exempt && !used.contains(&key) {
                used.push(key);
                continue;
            }
            findings.push(Finding {
                path: relative.clone(),
                name: name.to_owned(),
            });
        }
    }
    Ok(findings)
}

fn collect(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), XtaskError> {
    let entries = std::fs::read_dir(dir).map_err(|source| XtaskError::Io {
        context: "reading the Aviate adapter tree for the control-feel guard",
        source,
    })?;
    for entry in entries {
        let path = entry
            .map_err(|source| XtaskError::Io {
                context: "reading an Aviate adapter tree entry",
                source,
            })?
            .path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if path.is_dir() {
            if name != "tests" {
                collect(&path, files)?;
            }
        } else if name.ends_with(".rs") && name != "tests.rs" {
            files.push(path);
        }
    }
    Ok(())
}

/// Whether `line` declares a public `new_with_profile` or `install_profile`.
pub(crate) fn exposes_profile_entry(line: &str) -> bool {
    let mut words = line.split_whitespace();
    if words.next() != Some("pub") {
        return false;
    }
    let mut word = words.next();
    while matches!(word, Some("async" | "const")) {
        word = words.next();
    }
    if word != Some("fn") {
        return false;
    }
    let rest: String = words.collect::<Vec<_>>().join(" ");
    ["new_with_profile", "install_profile"].iter().any(|entry| {
        rest.strip_prefix(entry)
            .is_some_and(|after| after.trim_start().starts_with('('))
    })
}

/// The identifier and the type text of a `const`/`static` declaration
/// with an upper-case name; `None` for any other line.
pub(crate) fn declaration(line: &str) -> Option<(&str, &str)> {
    let mut rest = line.trim_start();
    if let Some(after) = rest.strip_prefix("pub") {
        let after = if let Some(scoped) = after.strip_prefix('(') {
            scoped.split_once(')')?.1
        } else {
            after
        };
        if !after.starts_with(char::is_whitespace) {
            return None;
        }
        rest = after.trim_start();
    }
    let rest = rest
        .strip_prefix("const")
        .or_else(|| rest.strip_prefix("static"))?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = rest.trim_start();
    let end = rest
        .find(|c: char| !(c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
        .unwrap_or(rest.len());
    let name = &rest[..end];
    if !name.starts_with(|c: char| c.is_ascii_uppercase()) {
        return None;
    }
    let type_text = rest[end..]
        .split_once(':')
        .map_or("", |(_, after)| after.split('=').next().unwrap_or_default());
    Some((name, type_text))
}

/// Whether a declaration's name or type reads as a flight-response value.
pub(crate) fn reads_as_response(name: &str, type_text: &str) -> bool {
    let named = name
        .split('_')
        .any(|token| RESPONSE_TOKENS.contains(&token))
        || name.ends_with("_MS");
    let typed = type_text
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(|word| RESPONSE_TYPES.contains(&word));
    named || typed
}
