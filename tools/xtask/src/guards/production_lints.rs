//! Production targets carry no panic-capable operation and no unsafe
//! code: clippy forbids `unsafe_code`, `unwrap_used`, `expect_used`, and
//! `panic` on every library and binary of the workspace, so a local
//! `allow` cannot exempt one.
//!
//! A crate that binds a C or C++ library cannot call it without unsafe
//! code. Such a crate is named in [`FFI_CRATES`] and is checked through
//! its manifest. Its registration names the one module
//! that may hold its unsafe code. Clippy denies `unsafe_code` in it
//! instead of forbidding it, so that module's `allow` stands, and this
//! guard keeps the exemption small: an `unsafe` anywhere else in the
//! crate is refused, and every `unsafe` in the module carries a
//! `// SAFETY:` comment directly above it or on its line.

use std::fmt;
use std::path::Path;
use std::process::Command;

use crate::error::XtaskError;

#[cfg(test)]
mod tests;

/// A crate that binds a foreign library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FfiCrate {
    /// Workspace package to exclude from the ordinary lint group, if a member.
    pub(crate) workspace_package: Option<&'static str>,
    /// Its directory, relative to the repository root.
    pub(crate) dir: &'static str,
    /// The feature that compiles its binding, if the binding is optional.
    pub(crate) feature: Option<&'static str>,
    /// The one source file, relative to [`Self::dir`], that may hold
    /// unsafe code.
    pub(crate) unsafe_module: &'static str,
}

/// Every crate that binds a foreign library.
pub(crate) const FFI_CRATES: &[FfiCrate] = &[
    FfiCrate {
        workspace_package: None,
        dir: "sim/jsbsim-sidecar",
        feature: Some("cpp"),
        unsafe_module: "src/engine/ffi.rs",
    },
    FfiCrate {
        workspace_package: Some("pilotage-dji-usb"),
        dir: "adapters/dji-usb",
        feature: None,
        unsafe_module: "src/ffi.rs",
    },
];

/// The panic-capable operations no production target may use.
const PANIC_LINTS: [&str; 3] = [
    "clippy::unwrap_used",
    "clippy::expect_used",
    "clippy::panic",
];

/// One `unsafe` the exemption does not cover.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Finding {
    pub(crate) path: String,
    pub(crate) line: usize,
    pub(crate) reason: &'static str,
}

impl fmt::Display for Finding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "FORBIDDEN: {}:{}: {}",
            self.path, self.line, self.reason
        )
    }
}

/// The clippy argument lists the guard runs: the workspace under a
/// forbid of unsafe code, then each FFI crate, through its own manifest,
/// under a deny of it. Both forbid the panic-capable operations.
pub(crate) fn clippy_invocations(ffi: &[FfiCrate]) -> Vec<Vec<String>> {
    let lint_args = |unsafe_level: &str| {
        let mut args = vec![
            "--".to_owned(),
            "-D".to_owned(),
            "warnings".to_owned(),
            unsafe_level.to_owned(),
            "unsafe_code".to_owned(),
        ];
        for lint in PANIC_LINTS {
            args.extend(["-F".to_owned(), lint.to_owned()]);
        }
        args
    };
    // Each workspace package is a primary target in one group. Dependency
    // compilation must not apply another group's unsafe-code lint level.
    let targets = ["--lib", "--bins", "--examples", "--no-deps"].map(str::to_owned);
    let mut workspace = vec!["clippy".to_owned(), "--workspace".to_owned()];
    workspace.extend(targets.clone());
    for krate in ffi {
        if let Some(package) = krate.workspace_package {
            workspace.extend(["--exclude".to_owned(), package.to_owned()]);
        }
    }
    workspace.extend(lint_args("-F"));
    let mut invocations = vec![workspace];
    for krate in ffi {
        let mut args = vec![
            "clippy".to_owned(),
            "--manifest-path".to_owned(),
            format!("{}/Cargo.toml", krate.dir),
        ];
        if let Some(feature) = krate.feature {
            args.extend(["--features".to_owned(), feature.to_owned()]);
        }
        args.extend(targets.clone());
        args.extend(lint_args("-D"));
        invocations.push(args);
    }
    invocations
}

/// Whether a line of code (comments removed) uses the `unsafe` keyword
/// or names the `unsafe_code` lint.
fn uses_unsafe(code: &str) -> bool {
    code.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|word| word == "unsafe" || word == "unsafe_code")
}

/// The `unsafe` uses in one file of an FFI crate that the exemption does
/// not cover. `is_unsafe_module` says whether this is the crate's one
/// unsafe module.
pub(crate) fn findings_in(path: &str, text: &str, is_unsafe_module: bool) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut justified = false;
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        let (code, comment) = match line.find("//") {
            Some(at) => (&line[..at], Some(&line[at..])),
            None => (line, None),
        };
        let safety_here = comment.is_some_and(|comment| comment.contains("SAFETY:"));
        if uses_unsafe(code) {
            if !is_unsafe_module {
                findings.push(Finding {
                    path: path.to_owned(),
                    line: index + 1,
                    reason: "unsafe code outside the crate's one unsafe module",
                });
            } else if !(justified || safety_here) {
                findings.push(Finding {
                    path: path.to_owned(),
                    line: index + 1,
                    reason: "unsafe code without a `// SAFETY:` comment directly above it",
                });
            }
        }
        // A SAFETY comment covers the code right below it; any line that
        // is neither a comment nor an attribute ends its reach.
        if trimmed.starts_with("//") {
            justified |= safety_here;
        } else if !trimmed.starts_with("#[") {
            justified = false;
        }
    }
    findings
}

fn rust_files(dir: &Path, files: &mut Vec<std::path::PathBuf>) -> Result<(), XtaskError> {
    let entries = std::fs::read_dir(dir).map_err(|source| XtaskError::Io {
        context: "reading an FFI crate directory for the production lint guard",
        source,
    })?;
    for entry in entries {
        let path = entry
            .map_err(|source| XtaskError::Io {
                context: "reading an FFI crate directory entry",
                source,
            })?
            .path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name != "target") {
                rust_files(&path, files)?;
            }
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
    }
    Ok(())
}

/// Every `unsafe` in the FFI crates that the exemption does not cover.
///
/// # Errors
///
/// Returns [`XtaskError::Io`] when a crate's sources cannot be read, or
/// [`XtaskError::MissingArtifact`] when its named unsafe module is gone.
pub(crate) fn unsafe_findings(
    repo_root: &Path,
    ffi: &[FfiCrate],
) -> Result<Vec<Finding>, XtaskError> {
    let mut findings = Vec::new();
    for krate in ffi {
        let dir = repo_root.join(krate.dir);
        let module = dir.join(krate.unsafe_module);
        if !module.is_file() {
            return Err(XtaskError::MissingArtifact {
                what: "the FFI crate's unsafe module",
                path: module,
                hint: "update FFI_CRATES in tools/xtask/src/guards/production_lints.rs",
            });
        }
        let mut files = Vec::new();
        rust_files(&dir, &mut files)?;
        files.sort();
        for file in files {
            let text = std::fs::read_to_string(&file).map_err(|source| XtaskError::Io {
                context: "reading an FFI crate source for the production lint guard",
                source,
            })?;
            let shown = file.strip_prefix(repo_root).unwrap_or(&file);
            findings.extend(findings_in(
                &shown.display().to_string(),
                &text,
                file == module,
            ));
        }
    }
    Ok(findings)
}

/// Runs the guard: the source scan, then every clippy invocation.
/// Returns one line per failure; none means the guard passed.
///
/// # Errors
///
/// Returns the scan's errors, or [`XtaskError::Spawn`] when cargo
/// cannot start.
pub(crate) fn check(repo_root: &Path) -> Result<Vec<String>, XtaskError> {
    let mut failures: Vec<String> = unsafe_findings(repo_root, FFI_CRATES)?
        .iter()
        .map(Finding::to_string)
        .collect();
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    for args in clippy_invocations(FFI_CRATES) {
        let status = Command::new(&cargo)
            .args(&args)
            .current_dir(repo_root)
            .status()
            .map_err(|source| XtaskError::Spawn {
                name: "cargo clippy",
                source,
            })?;
        if !status.success() {
            failures.push(format!(
                "FORBIDDEN: cargo {} failed ({status})",
                args.join(" ")
            ));
        }
    }
    Ok(failures)
}
