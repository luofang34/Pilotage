//! Contract tests for the backend registry and the host environment
//! the launcher assembles.

#![allow(clippy::expect_used, clippy::panic)]

use super::backend_for;
use crate::error::XtaskError;

/// Every Aviate backend must name a control-feel law belonging to the
/// aircraft it actually launches.
///
/// The host cannot check this for itself: both backends hand it
/// `--adapter aviate`, so absent an explicit name it loads one
/// compiled-in default for every vehicle — a law qualified for a
/// different airframe. The launcher is the only component that knows
/// which aircraft it started, so this is where the binding is asserted.
#[test]
fn each_aviate_backend_names_its_own_vehicles_control_feel() {
    use std::path::{Path, PathBuf};

    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root is two levels above tools/xtask")
        .to_path_buf();

    // Backend name -> the substring its law's file name must carry.
    for (backend, vehicle) in [("aviate-gz", "x500"), ("aviate-xplane", "alia250")] {
        let declared = backend_for(backend)
            .expect("known backend")
            .control_feel_profile()
            .unwrap_or_else(|| {
                panic!(
                    "{backend} names no control-feel law, so the host falls back to \
                         whatever single default it was compiled with"
                )
            });

        let path = repo_root.join(declared);
        let name = path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or_default();
        assert!(
            name.starts_with(vehicle),
            "{backend} launches {vehicle} but names the control-feel law {name:?}, \
                 which is another aircraft's"
        );
        assert!(
            path.is_file(),
            "{backend} names a control-feel law at {} that does not exist",
            path.display()
        );
    }
}

/// The environment the launcher ASSEMBLES carries the vehicle's law.
///
/// Asserted against the STAGE the launcher pushes, not against the
/// composition function it calls. Both halves being right does not
/// make the join right, and a test of the join itself still passes
/// while a caller assembles the right environment and hands the host
/// a different one. The stage is the last artifact before the process
/// is spawned, so there is nothing left between it and the host.
///
/// Exactly one entry, not merely a correct one — this composition is
/// where a duplicate could appear, and the last write is the one that
/// reaches the host.
#[test]
fn the_assembled_host_environment_carries_the_vehicles_law() {
    use super::{BACKENDS, CONTROL_FEEL_ENV, SessionContext};
    use crate::cli::Profile;
    use crate::session::host_stage;
    use std::path::{Path, PathBuf};

    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root is two levels above tools/xtask")
        .to_path_buf();
    let ctx_for = |profile| SessionContext {
        repo_root: repo_root.clone(),
        host_port: 4433,
        viewer_port: 8080,
        profile,
        log_dir: repo_root.join("target/xtask-sim"),
        lan: false,
    };

    for (backend, vehicle) in [("aviate-gz", "x500"), ("aviate-xplane", "alia250")] {
        let sim = ctx_for(Profile::Simulation);
        let stage = host_stage(&sim, backend_for(backend).expect("known backend").as_ref());
        let env = &stage.spec.env;
        let named: Vec<_> = env
            .iter()
            .filter(|(key, _)| key == CONTROL_FEEL_ENV)
            .collect();
        assert_eq!(
            named.len(),
            1,
            "the environment {backend} hands the host names the control-feel law \
                 {} times; the host reads one value and the last write wins",
            named.len()
        );
        let path = PathBuf::from(named[0].1.clone());
        let name = path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or_default();
        assert!(
            name.starts_with(vehicle),
            "{backend} launches {vehicle} but hands the host {name:?}, which is \
                 another aircraft's law"
        );
        assert!(path.is_file(), "{} does not exist", path.display());

        let physical = ctx_for(Profile::Physical);
        let stage = host_stage(
            &physical,
            backend_for(backend).expect("known backend").as_ref(),
        );
        let env = &stage.spec.env;
        assert!(
            !env.iter().any(|(key, _)| key == CONTROL_FEEL_ENV),
            "{backend} hands a physical session a control-feel law, which the host \
                 refuses with AviatePhysicalControlFeelOverride"
        );
    }

    // Every registered backend, so a new one cannot opt out of the
    // composition by being absent from a hand-written list.
    for entry in BACKENDS {
        let stage = host_stage(
            &ctx_for(Profile::Simulation),
            backend_for(entry.name).expect("registered").as_ref(),
        );
        let env = &stage.spec.env;
        assert!(
            env.iter().filter(|(k, _)| k == CONTROL_FEEL_ENV).count() <= 1,
            "{} assembles more than one control-feel law",
            entry.name
        );
    }
}

/// No backend may write the control-feel variable itself.
///
/// A backend that assembles it by hand has to remember every case
/// where passing a law does harm — a physical session, and an operator
/// who already named one — and one that remembers only the first
/// silently replaces the operator's law while every other test here
/// still passes. Declaring the law and writing the variable are
/// therefore separate: backends return a path, and the launcher is the
/// only writer.
#[test]
fn no_backend_writes_the_control_feel_variable_itself() {
    use super::{CONTROL_FEEL_ENV, SessionContext};
    use crate::cli::Profile;
    use std::path::PathBuf;

    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for entry in super::BACKENDS {
        let backend = entry.name;
        for profile in [Profile::Simulation, Profile::Physical, Profile::OracleOnly] {
            let ctx = SessionContext {
                repo_root: repo_root.clone(),
                host_port: 4433,
                viewer_port: 8080,
                profile,
                log_dir: repo_root.join("target/xtask-sim"),
                lan: false,
            };
            let env = backend_for(backend).expect("known backend").host_env(&ctx);
            assert!(
                !env.iter().any(|(key, _)| key == CONTROL_FEEL_ENV),
                "{backend} writes {CONTROL_FEEL_ENV} in host_env for {profile:?}; \
                     it must declare the law through control_feel_profile() and let the \
                     launcher decide whether passing it is safe"
            );
        }
    }
}

/// A physical session must be handed NO control-feel law.
///
/// The host does not ignore one there, it refuses the session
/// outright, so a launcher that names a law unconditionally turns a
/// working `--profile physical` run into a startup failure. Naming a
/// law per vehicle is what creates that risk, and this test is what
/// holds it closed.
#[test]
fn a_physical_session_is_handed_no_control_feel_law() {
    use super::SessionContext;
    use crate::cli::Profile;
    use std::path::PathBuf;

    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for backend in ["aviate-gz", "aviate-xplane"] {
        let ctx = SessionContext {
            repo_root: repo_root.clone(),
            host_port: 4433,
            viewer_port: 8080,
            profile: Profile::Physical,
            log_dir: repo_root.join("target/xtask-sim"),
            lan: false,
        };
        let declared = backend_for(backend)
            .expect("known backend")
            .control_feel_profile()
            .expect("an Aviate backend declares a law");
        assert!(
            super::control_feel_env(&ctx, declared).is_empty(),
            "{backend} names a control-feel law for a physical session, which the \
                 host refuses with AviatePhysicalControlFeelOverride"
        );
    }
}

/// An operator who already names a law keeps it.
///
/// Stage environment is applied on top of what the launcher inherited,
/// so a value written by a backend REPLACES the operator's with no
/// diagnostic on either side: the vehicle flies a law nobody chose and
/// nothing says so. That is the same failure this whole change exists
/// to prevent, pointed the other way.
#[test]
fn an_operators_own_control_feel_law_is_not_replaced() {
    use super::{CONTROL_FEEL_ENV, SessionContext, control_feel_env_given};
    use crate::cli::Profile;
    use std::path::PathBuf;

    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ctx = SessionContext {
        repo_root: repo_root.clone(),
        host_port: 4433,
        viewer_port: 8080,
        profile: Profile::Simulation,
        log_dir: repo_root.join("target/xtask-sim"),
        lan: false,
    };

    let operators = std::ffi::OsString::from("/tmp/an-operators-own-tuning.json");
    assert!(
        control_feel_env_given(
            &ctx,
            "adapters/aviate/profiles/x500-shaped-balanced-v1.json",
            Some(operators)
        )
        .is_empty(),
        "the launcher writes {CONTROL_FEEL_ENV} over the operator's own value, which \
             the child inherits and would otherwise have used"
    );

    // The same call with nothing ambient must still bind, or the guard
    // above would be satisfied by never naming a law at all.
    assert_eq!(
        control_feel_env_given(
            &ctx,
            "adapters/aviate/profiles/x500-shaped-balanced-v1.json",
            None
        )
        .len(),
        1,
        "with no operator value the launcher must name the vehicle's law"
    );
}

/// The name a backend is registered under is the name it answers to.
///
/// `--fc` resolves through the table while everything the operator
/// reads — the launching line, a stale-session refusal — comes from
/// `name()`. A disagreement makes the launcher report one backend
/// while running another, and nothing else compares the two.
#[test]
fn every_registered_backend_answers_to_the_name_it_is_registered_under() {
    for entry in super::BACKENDS {
        let resolved = backend_for(entry.name).expect("registered name resolves");
        assert_eq!(
            resolved.name(),
            entry.name,
            "the table registers {:?} but the backend it builds calls itself {:?}",
            entry.name,
            resolved.name()
        );
        if let Some(alias) = entry.alias {
            assert_eq!(
                backend_for(alias).expect("alias resolves").name(),
                entry.name,
                "alias {alias:?} resolves to a backend that is not {:?}",
                entry.name
            );
        }
    }
}

#[test]
fn backend_selection_fails_closed() {
    assert_eq!(backend_for("aviate").expect("known").name(), "aviate-gz");
    assert_eq!(backend_for("aviate-gz").expect("known").name(), "aviate-gz");
    assert_eq!(backend_for("px4").expect("known").name(), "px4-gz");
    assert_eq!(backend_for("px4-gz").expect("known").name(), "px4-gz");
    assert_eq!(
        backend_for("px4-xplane").expect("known").name(),
        "px4-xplane"
    );
    assert_eq!(
        backend_for("aviate-xplane").expect("known").name(),
        "aviate-xplane"
    );
    let refusal = backend_for("px4-jsbsim");
    assert!(matches!(refusal, Err(XtaskError::UnknownBackend { .. })));
}
