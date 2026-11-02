//! Locating, validating, and planning the build of the pinned JSBSim.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};

use super::{
    BUILD_SCRIPT, JSBSIM_TAG, JsbsimInstall, SidecarEngine, cpp_chosen, jsbsim_dir_from, pc_version,
};
use crate::error::XtaskError;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("plt_xt_jsbsim_{tag}_{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Lays out what a finished `build-jsbsim` leaves behind, at `version`.
fn fake_install(root: &Path, version: &str) {
    for dir in [
        "aircraft",
        "engine",
        "install/include/JSBSim",
        "install/lib/pkgconfig",
    ] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    std::fs::write(root.join("install/include/JSBSim/FGFDMExec.h"), "").unwrap();
    std::fs::write(root.join("install/lib/libJSBSim.a"), "").unwrap();
    std::fs::write(
        root.join("install/lib/pkgconfig/JSBSim.pc"),
        format!("Name: JSBSim\nVersion: {version}\nLibs: -lJSBSim\n"),
    )
    .unwrap();
}

#[test]
fn jsbsim_dir_prefers_the_variable_then_the_home_directory() {
    assert_eq!(
        jsbsim_dir_from(Some("/opt/jsbsim".into()), Some("/home/u".into())).unwrap(),
        PathBuf::from("/opt/jsbsim")
    );
    assert_eq!(
        jsbsim_dir_from(None, Some("/home/u".into())).unwrap(),
        PathBuf::from("/home/u/jsbsim")
    );
    // An empty variable is unset, not the current directory.
    assert_eq!(
        jsbsim_dir_from(Some("".into()), Some("/home/u".into())).unwrap(),
        PathBuf::from("/home/u/jsbsim")
    );
    assert!(matches!(
        jsbsim_dir_from(None, None),
        Err(XtaskError::Usage { .. })
    ));
}

#[test]
fn a_complete_install_of_the_pinned_release_validates() {
    let root = scratch("complete");
    fake_install(&root, "1.3.3");
    assert_eq!(JsbsimInstall::validate(&root).unwrap().root, root);
}

#[test]
fn each_missing_piece_is_named_with_the_build_command() {
    let root = scratch("missing");
    fake_install(&root, "1.3.3");
    std::fs::remove_file(root.join("install/lib/libJSBSim.a")).unwrap();
    match JsbsimInstall::validate(&root) {
        Err(XtaskError::MissingArtifact { what, path, hint }) => {
            assert_eq!(what, "JSBSim static library");
            assert_eq!(path, root.join("install/lib/libJSBSim.a"));
            assert!(hint.contains("cargo xtask build-jsbsim"), "{hint}");
        }
        other => panic!("expected a missing library, got {other:?}"),
    }
    let empty = scratch("empty");
    assert!(matches!(
        JsbsimInstall::validate(&empty),
        Err(XtaskError::MissingArtifact {
            what: "JSBSim aircraft data",
            ..
        })
    ));
}

#[test]
fn an_install_of_another_release_is_refused() {
    let root = scratch("other_release");
    fake_install(&root, "1.2.4");
    assert!(matches!(
        JsbsimInstall::validate(&root),
        Err(XtaskError::MissingArtifact {
            what: "JSBSim build of the pinned release",
            ..
        })
    ));
}

#[test]
fn the_pc_version_is_the_version_field() {
    assert_eq!(pc_version("Name: JSBSim\nVersion: 1.3.3\n"), Some("1.3.3"));
    assert_eq!(pc_version("Name: JSBSim\n"), None);
}

#[test]
fn the_build_script_builds_the_pinned_release() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let script = std::fs::read_to_string(repo_root.join(BUILD_SCRIPT)).unwrap();
    assert!(
        script.contains(&format!("JSBSIM_TAG=\"{JSBSIM_TAG}\"")),
        "{BUILD_SCRIPT} must pin {JSBSIM_TAG}"
    );
    for flag in [
        "-DBUILD_SHARED_LIBS=OFF",
        "-DBUILD_PYTHON_MODULE=OFF",
        "-DCMAKE_INSTALL_PREFIX=$root/install",
    ] {
        assert!(script.contains(flag), "missing {flag}");
    }
}

/// The world-model revs a manifest pins, one per `world-*` dependency.
fn world_revs(manifest: &str) -> Vec<String> {
    manifest
        .lines()
        .filter(|line| line.starts_with("world-"))
        .map(|line| {
            let rev = line.split("rev = \"").nth(1).unwrap();
            rev.split('"').next().unwrap().to_owned()
        })
        .collect()
}

#[test]
fn the_sidecar_builds_alone_on_the_workspace_world_rev() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = std::fs::read_to_string(repo_root.join("Cargo.toml")).unwrap();
    let sidecar = std::fs::read_to_string(repo_root.join("sim/jsbsim-sidecar/Cargo.toml")).unwrap();
    assert!(
        root.contains("exclude = [\"sim/jsbsim-sidecar\""),
        "the sidecar must stay out of the workspace"
    );
    assert!(sidecar.contains("\n[workspace]\n"));
    let workspace_revs = world_revs(&root);
    assert!(!workspace_revs.is_empty());
    let sidecar_revs = world_revs(&sidecar);
    assert!(!sidecar_revs.is_empty());
    for rev in workspace_revs.iter().chain(&sidecar_revs) {
        assert_eq!(rev, &workspace_revs[0], "one world rev everywhere");
    }
    assert!(
        !sidecar.contains("workspace = true"),
        "the sidecar inherits nothing from the workspace"
    );
}

#[test]
fn the_rust_port_flies_unless_the_cpp_library_is_chosen() {
    assert!(!cpp_chosen(None).unwrap());
    assert!(!cpp_chosen(Some("".into())).unwrap());
    assert!(!cpp_chosen(Some("rust".into())).unwrap());
    assert!(cpp_chosen(Some("cpp".into())).unwrap());
    assert!(matches!(
        cpp_chosen(Some("fortran".into())),
        Err(XtaskError::Usage { .. })
    ));
}

#[test]
fn only_the_cpp_library_needs_a_checkout_on_the_sidecars_command_line() {
    assert!(SidecarEngine::Rust.sidecar_args().is_empty());
    let cpp = SidecarEngine::Cpp(JsbsimInstall {
        root: "/opt/jsbsim".into(),
    });
    assert_eq!(
        cpp.sidecar_args(),
        ["--engine", "cpp", "--jsbsim-root", "/opt/jsbsim"]
    );
}
