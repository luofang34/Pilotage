//! Which JSBSim the sidecar flies. The pure-Rust port is a dependency of
//! the sidecar and needs nothing here. `JSBSIM_ENGINE=cpp` chooses
//! JSBSim's C++ library instead: where its checkout lives, which release
//! it must be, and how to build it.
//!
//! The C++ library is an external checkout, like PX4-Autopilot and
//! ArduPilot. The checkout holds the aircraft, engine, and systems data
//! the sidecar loads at run time; its `install` tree holds the static
//! library and headers the sidecar links at build time. `cargo xtask
//! build-jsbsim` produces both from the pinned release with
//! `scripts/build-jsbsim.sh`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::XtaskError;
use crate::output::print_line;

/// The JSBSim release the sidecar is written against.
pub(crate) const JSBSIM_TAG: &str = "v1.3.3";
/// The version `JSBSim.pc` states for [`JSBSIM_TAG`].
const JSBSIM_VERSION: &str = "1.3.3";

/// Where the JSBSim checkout lives: `JSBSIM_DIR`, else `jsbsim` in the
/// home directory. Not a directory next to this repository: a worktree
/// named `jsbsim` would then name itself.
///
/// # Errors
///
/// Returns [`XtaskError::Usage`] when neither variable is set.
pub(crate) fn jsbsim_dir() -> Result<PathBuf, XtaskError> {
    jsbsim_dir_from(std::env::var_os("JSBSIM_DIR"), std::env::var_os("HOME"))
}

fn jsbsim_dir_from(
    jsbsim_dir: Option<OsString>,
    home: Option<OsString>,
) -> Result<PathBuf, XtaskError> {
    match (jsbsim_dir, home) {
        (Some(dir), _) if !dir.is_empty() => Ok(PathBuf::from(dir)),
        (_, Some(home)) if !home.is_empty() => Ok(PathBuf::from(home).join("jsbsim")),
        _ => Err(XtaskError::Usage {
            message: "set JSBSIM_DIR to the JSBSim checkout (HOME is not set either)".to_owned(),
        }),
    }
}

/// The implementation of JSBSim a session flies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SidecarEngine {
    /// The pure-Rust port.
    Rust,
    /// The C++ library of a validated checkout.
    Cpp(JsbsimInstall),
}

/// Whether `JSBSIM_ENGINE` names the C++ library. Unset or `rust` is the
/// Rust port.
fn cpp_chosen(engine: Option<OsString>) -> Result<bool, XtaskError> {
    let Some(engine) = engine else {
        return Ok(false);
    };
    match engine.to_str() {
        Some("" | "rust") => Ok(false),
        Some("cpp") => Ok(true),
        _ => Err(XtaskError::Usage {
            message: format!("JSBSIM_ENGINE must be rust or cpp (got {engine:?})"),
        }),
    }
}

impl SidecarEngine {
    /// Whether this session chose the C++ library.
    ///
    /// # Errors
    ///
    /// Returns [`XtaskError::Usage`] for another `JSBSIM_ENGINE`.
    pub(crate) fn cpp_selected() -> Result<bool, XtaskError> {
        cpp_chosen(std::env::var_os("JSBSIM_ENGINE"))
    }

    /// The engine this session chose, validated.
    ///
    /// # Errors
    ///
    /// Returns [`XtaskError::Usage`] for another `JSBSIM_ENGINE`, and the
    /// errors of [`jsbsim_dir`] and [`JsbsimInstall::validate`] for the
    /// C++ library.
    pub(crate) fn select() -> Result<Self, XtaskError> {
        if Self::cpp_selected()? {
            Ok(Self::Cpp(JsbsimInstall::validate(&jsbsim_dir()?)?))
        } else {
            Ok(Self::Rust)
        }
    }

    /// The sidecar's flags for this engine.
    pub(crate) fn sidecar_args(&self) -> Vec<String> {
        match self {
            Self::Rust => vec![],
            Self::Cpp(install) => vec![
                "--engine".to_owned(),
                "cpp".to_owned(),
                "--jsbsim-root".to_owned(),
                install.root.display().to_string(),
            ],
        }
    }
}

/// A validated JSBSim checkout with its installed build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JsbsimInstall {
    /// The checkout: the root the sidecar loads aircraft data from.
    pub(crate) root: PathBuf,
}

impl JsbsimInstall {
    /// The installed build inside the checkout.
    pub(crate) fn prefix(root: &Path) -> PathBuf {
        root.join("install")
    }

    /// Checks that `root` holds the run-time data and an installed build
    /// of [`JSBSIM_TAG`].
    ///
    /// # Errors
    ///
    /// Returns [`XtaskError::MissingArtifact`] naming the first absent
    /// piece, with the command that produces it.
    pub(crate) fn validate(root: &Path) -> Result<Self, XtaskError> {
        const BUILD_HINT: &str = "build it: cargo xtask build-jsbsim (or set JSBSIM_DIR)";
        let prefix = Self::prefix(root);
        let required: [(&'static str, PathBuf); 5] = [
            ("JSBSim aircraft data", root.join("aircraft")),
            ("JSBSim engine data", root.join("engine")),
            ("JSBSim headers", prefix.join("include/JSBSim/FGFDMExec.h")),
            ("JSBSim static library", prefix.join("lib/libJSBSim.a")),
            (
                "JSBSim pkg-config file",
                prefix.join("lib/pkgconfig/JSBSim.pc"),
            ),
        ];
        for (what, path) in required {
            if !path.exists() {
                return Err(XtaskError::MissingArtifact {
                    what,
                    path,
                    hint: BUILD_HINT,
                });
            }
        }
        let pc_path = prefix.join("lib/pkgconfig/JSBSim.pc");
        let pc = std::fs::read_to_string(&pc_path).map_err(|source| XtaskError::Io {
            context: "reading the JSBSim pkg-config file",
            source,
        })?;
        if pc_version(&pc) != Some(JSBSIM_VERSION) {
            return Err(XtaskError::MissingArtifact {
                what: "JSBSim build of the pinned release",
                path: pc_path,
                hint: "the installed JSBSim is another release; remove the checkout \
                       and run cargo xtask build-jsbsim",
            });
        }
        Ok(Self {
            root: root.to_path_buf(),
        })
    }
}

/// The `Version:` field of a pkg-config file.
fn pc_version(pc: &str) -> Option<&str> {
    pc.lines()
        .find_map(|line| line.strip_prefix("Version:"))
        .map(str::trim)
}

/// The script that clones, configures, builds, and installs
/// [`JSBSIM_TAG`], relative to the repository root. It is the one build
/// recipe: a checkout that cannot run xtask runs it directly.
pub(crate) const BUILD_SCRIPT: &str = "scripts/build-jsbsim.sh";

/// `cargo xtask build-jsbsim`: runs [`BUILD_SCRIPT`] for the checkout at
/// [`jsbsim_dir`], then validates the result.
///
/// # Errors
///
/// Returns [`XtaskError::CommandFailed`] for a failed build, and the
/// errors of [`jsbsim_dir`] and [`JsbsimInstall::validate`].
pub(crate) fn run_build_jsbsim(repo_root: &Path) -> Result<(), XtaskError> {
    let root = jsbsim_dir()?;
    let status = Command::new("bash")
        .arg(repo_root.join(BUILD_SCRIPT))
        .env("JSBSIM_DIR", &root)
        .status()
        .map_err(|source| XtaskError::Spawn {
            name: "bash",
            source,
        })?;
    if !status.success() {
        return Err(XtaskError::CommandFailed {
            context: "building JSBSim (scripts/build-jsbsim.sh)",
            status: status.to_string(),
        });
    }
    let install = JsbsimInstall::validate(&root)?;
    print_line(&format!(
        "JSBSim {JSBSIM_TAG} is installed in {}",
        JsbsimInstall::prefix(&install.root).display()
    ));
    Ok(())
}

#[cfg(test)]
mod tests;
