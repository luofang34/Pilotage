//! The JSBSim flight-dynamics library the JSBSim sidecar links: where its
//! checkout lives, which release it must be, and how to build it.
//!
//! JSBSim is an external checkout, like PX4-Autopilot and ArduPilot. The
//! checkout holds the aircraft, engine, and systems data the sidecar
//! loads at run time; its `install` tree holds the static library and
//! headers the sidecar links at build time. `cargo xtask build-jsbsim`
//! produces both from the pinned release with CMake, without Python.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::XtaskError;
use crate::output::print_line;

/// The JSBSim release the sidecar is written against.
pub(crate) const JSBSIM_TAG: &str = "v1.3.3";
/// The version `JSBSim.pc` states for [`JSBSIM_TAG`].
const JSBSIM_VERSION: &str = "1.3.3";
/// Where the release is cloned from.
const JSBSIM_REPOSITORY: &str = "https://github.com/JSBSim-Team/jsbsim.git";

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

/// One command of the build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BuildStep {
    /// What the step does, for progress and errors.
    pub(crate) context: &'static str,
    /// The program.
    pub(crate) program: &'static str,
    /// Its arguments.
    pub(crate) args: Vec<String>,
}

/// The commands that clone (when `cloned` is false), configure, build,
/// and install [`JSBSIM_TAG`] at `root`. The build is a static library
/// with no Python module and no documentation, so it needs only CMake
/// and a C++ compiler.
pub(crate) fn build_plan(root: &Path, cloned: bool, jobs: usize) -> Vec<BuildStep> {
    let root_arg = root.display().to_string();
    let build_dir = root.join("build").display().to_string();
    let mut steps = Vec::new();
    if !cloned {
        steps.push(BuildStep {
            context: "cloning JSBSim",
            program: "git",
            args: [
                "clone",
                "--depth",
                "1",
                "--branch",
                JSBSIM_TAG,
                JSBSIM_REPOSITORY,
                &root_arg,
            ]
            .map(str::to_owned)
            .to_vec(),
        });
    }
    steps.push(BuildStep {
        context: "configuring JSBSim",
        program: "cmake",
        args: vec![
            "-S".to_owned(),
            root_arg,
            "-B".to_owned(),
            build_dir.clone(),
            "-DCMAKE_BUILD_TYPE=Release".to_owned(),
            "-DBUILD_SHARED_LIBS=OFF".to_owned(),
            "-DBUILD_PYTHON_MODULE=OFF".to_owned(),
            "-DBUILD_DOCS=OFF".to_owned(),
            // Position-independent code, so a Linux link of the static
            // library into a Rust binary does not need text relocations.
            "-DCMAKE_POSITION_INDEPENDENT_CODE=ON".to_owned(),
            format!(
                "-DCMAKE_INSTALL_PREFIX={}",
                JsbsimInstall::prefix(root).display()
            ),
        ],
    });
    steps.push(BuildStep {
        context: "building and installing JSBSim",
        program: "cmake",
        args: vec![
            "--build".to_owned(),
            build_dir,
            "--target".to_owned(),
            "install".to_owned(),
            "--parallel".to_owned(),
            jobs.to_string(),
        ],
    });
    steps
}

/// Checks whether `root` is a checkout of [`JSBSIM_TAG`]: a directory
/// that holds JSBSim's CMake project. An empty or absent directory is
/// not cloned yet.
fn checkout_state(root: &Path) -> Result<bool, XtaskError> {
    if root.join("CMakeLists.txt").is_file() && root.join("src/FGFDMExec.h").is_file() {
        return Ok(true);
    }
    let empty = match std::fs::read_dir(root) {
        Ok(mut entries) => entries.next().is_none(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(source) => {
            return Err(XtaskError::Io {
                context: "reading the JSBSim directory",
                source,
            });
        }
    };
    if empty {
        Ok(false)
    } else {
        Err(XtaskError::MissingArtifact {
            what: "JSBSim checkout",
            path: root.to_path_buf(),
            hint: "the directory is not a JSBSim checkout; point JSBSIM_DIR at an empty \
                   or absent directory",
        })
    }
}

/// `cargo xtask build-jsbsim`: clones the pinned release when absent,
/// then configures, builds, and installs it, and validates the result.
///
/// # Errors
///
/// Returns [`XtaskError::CommandFailed`] for a failed step, and the
/// errors of [`jsbsim_dir`] and [`JsbsimInstall::validate`].
pub(crate) fn run_build_jsbsim() -> Result<(), XtaskError> {
    let root = jsbsim_dir()?;
    let cloned = checkout_state(&root)?;
    let jobs = std::thread::available_parallelism().map_or(2, usize::from);
    for step in build_plan(&root, cloned, jobs) {
        print_line(&format!(
            "{}: {} {}",
            step.context,
            step.program,
            step.args.join(" ")
        ));
        let status = Command::new(step.program)
            .args(&step.args)
            .status()
            .map_err(|source| XtaskError::Spawn {
                name: step.program,
                source,
            })?;
        if !status.success() {
            return Err(XtaskError::CommandFailed {
                context: step.context,
                status: status.to_string(),
            });
        }
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
