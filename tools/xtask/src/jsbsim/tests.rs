//! Locating, validating, and planning the build of the pinned JSBSim.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};

use super::{JSBSIM_TAG, JsbsimInstall, build_plan, checkout_state, jsbsim_dir_from, pc_version};
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
fn the_plan_clones_the_pinned_tag_only_when_absent() {
    let root = Path::new("/home/u/jsbsim");
    let fresh = build_plan(root, false, 8);
    assert_eq!(fresh.len(), 3);
    assert_eq!(fresh[0].program, "git");
    let tag_at = fresh[0]
        .args
        .iter()
        .position(|arg| arg == "--branch")
        .unwrap();
    assert_eq!(fresh[0].args[tag_at + 1], JSBSIM_TAG);
    assert_eq!(fresh[0].args.last().unwrap(), "/home/u/jsbsim");
    let cloned = build_plan(root, true, 8);
    assert_eq!(cloned.len(), 2);
    assert_eq!(cloned, fresh[1..]);
}

#[test]
fn the_plan_builds_a_static_library_without_python_into_the_install_tree() {
    let plan = build_plan(Path::new("/home/u/jsbsim"), true, 6);
    let configure = &plan[0].args;
    for flag in [
        "-DBUILD_SHARED_LIBS=OFF",
        "-DBUILD_PYTHON_MODULE=OFF",
        "-DBUILD_DOCS=OFF",
        "-DCMAKE_BUILD_TYPE=Release",
        "-DCMAKE_INSTALL_PREFIX=/home/u/jsbsim/install",
    ] {
        assert!(configure.iter().any(|arg| arg == flag), "missing {flag}");
    }
    assert_eq!(
        plan[1].args,
        [
            "--build",
            "/home/u/jsbsim/build",
            "--target",
            "install",
            "--parallel",
            "6"
        ]
    );
}

#[test]
fn a_checkout_is_detected_and_a_foreign_directory_is_refused() {
    let absent = scratch("absent").join("jsbsim");
    assert!(!checkout_state(&absent).unwrap());
    let empty = scratch("empty_dir");
    assert!(!checkout_state(&empty).unwrap());
    let checkout = scratch("checkout");
    std::fs::create_dir_all(checkout.join("src")).unwrap();
    std::fs::write(checkout.join("CMakeLists.txt"), "").unwrap();
    std::fs::write(checkout.join("src/FGFDMExec.h"), "").unwrap();
    assert!(checkout_state(&checkout).unwrap());
    let foreign = scratch("foreign");
    std::fs::write(foreign.join("Cargo.toml"), "").unwrap();
    assert!(matches!(
        checkout_state(&foreign),
        Err(XtaskError::MissingArtifact {
            what: "JSBSim checkout",
            ..
        })
    ));
}
