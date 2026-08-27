//! Everything a session settles before it has children: which manifest
//! wins, which port was actually taken, what the viewer serves.

#![allow(clippy::expect_used, clippy::panic)]

use super::*;

#[test]
fn compiled_manifest_is_the_outside_cargo_fallback() {
    let compiled = Path::new("/compiled/Pilotage/tools/xtask");

    assert_eq!(resolve_manifest(None, compiled), compiled);
}

/// Artifacts alone no longer short-circuit the preflight: without a
/// recorded content stamp the build reruns (fail closed toward rebuilding),
/// and only a stamp matching the current sources + toolchain skips it. The
/// temp root has no `scripts/`, so an attempted build fails loudly — `Ok`
/// proves nothing ran.
#[test]
fn web_assets_preflight_skips_only_with_a_matching_content_stamp() {
    let root = std::env::temp_dir().join(format!("plt_xt_web_ok_{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("temp root is created");
    // A real (empty) git repository, so the content stamp is computable.
    assert!(
        std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(&root)
            .status()
            .expect("git init runs")
            .success(),
        "git init succeeds"
    );
    for rel in WEB_RUNTIME_ARTIFACTS {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().expect("artifact has a parent"))
            .expect("artifact dir is created");
        std::fs::write(&path, b"generated").expect("artifact is written");
    }

    // Present artifacts WITHOUT a stamp rebuild — and the missing build
    // script surfaces that as a typed failure.
    let unstamped = prepare_web_assets(&root);
    assert!(
        matches!(unstamped, Err(XtaskError::CommandFailed { .. })),
        "artifacts without a stamp must rebuild, got {unstamped:?}"
    );

    // Recording the CURRENT stamp makes the same checkout skip the build.
    let sources = crate::session::preflight::web_runtime_sources(&root);
    let source_refs: Vec<&str> = sources.iter().map(String::as_str).collect();
    let current = crate::session::preflight::stamp::source_stamp(
        &root,
        &source_refs,
        &[&["rustc", "--version"], &["wasm-bindgen", "--version"]],
    )
    .expect("stamp computes inside a git repository");
    crate::session::preflight::stamp::write_stamp(
        &root.join("target/xtask-stamps/web-runtime.stamp"),
        &current,
    );
    let outcome = prepare_web_assets(&root);
    std::fs::remove_dir_all(&root).ok();

    assert!(
        outcome.is_ok(),
        "a matching content stamp skips the build, got {outcome:?}"
    );
}

/// A checkout missing the generated web runtime triggers a build. With no
/// `scripts/build-web-instruments.sh` under the temp root the build cannot
/// run, and the failure is surfaced as a typed fault — never silently served
/// as a dead viewer.
#[test]
fn web_assets_preflight_builds_and_surfaces_failure_when_absent() {
    let root = std::env::temp_dir().join(format!("plt_xt_web_missing_{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("temp root is created");

    let outcome = prepare_web_assets(&root);
    std::fs::remove_dir_all(&root).ok();

    assert!(
        matches!(outcome, Err(XtaskError::CommandFailed { .. })),
        "a missing web runtime must trigger a build whose failure is typed, got {outcome:?}"
    );
}

#[test]
fn listening_port_must_match_the_requested_port() {
    assert!(verify_listening_port(4433, 4433).is_ok());
    assert!(matches!(
        verify_listening_port(4433, 4434),
        Err(XtaskError::PortMismatch {
            requested: 4433,
            actual: 4434,
        })
    ));
}

/// Every viewer response must carry `Cache-Control: no-store` so the
/// browser can never present a cached client that has diverged from the
/// working-tree source. Spawns the real `viewer_stage` process on an
/// ephemeral port and reads the header off the wire.
#[tokio::test]
async fn viewer_server_sets_cache_control_no_store() {
    let repo_root = std::env::temp_dir().join(format!("plt_xt_viewer_{}", std::process::id()));
    let web = repo_root.join("clients/web");
    std::fs::create_dir_all(&web).expect("temp clients/web is created");
    std::fs::write(web.join("index.html"), "<title>viewer</title>")
        .expect("the viewer entrypoint is written");

    let ctx = SessionContext {
        repo_root: repo_root.clone(),
        host_port: 0,
        viewer_port: free_port(),
        profile: Profile::Simulation,
        log_dir: repo_root.clone(),
        lan: false,
    };
    let stage = viewer_stage(&ctx).expect("the viewer stage plans");
    let mut child = ManagedChild::spawn(&stage.spec).expect("the viewer server spawns");
    let ready = await_ready(&mut child, &stage.readiness).await;

    // Read the header off the wire only once the server accepts, but
    // always tear the process group down before asserting.
    let response = ready.is_ok().then(|| http_get(ctx.viewer_port, "/"));
    child.terminate_group();
    std::fs::remove_dir_all(&repo_root).ok();

    ready.expect("the viewer server accepts connections");
    let headers = response
        .unwrap_or_default()
        .split("\r\n\r\n")
        .next()
        .unwrap_or_default()
        .to_owned();
    assert!(
        headers
            .to_ascii_lowercase()
            .contains("cache-control: no-store"),
        "every viewer response must carry Cache-Control: no-store; got headers:\n{headers}"
    );
}

/// The wasm stamp's input set must be the DERIVED dependency closure, not a
/// hand-maintained list: the shared protocol and input engines (transitive
/// dependencies of the wasm crates) and the workspace manifests are all
/// inputs — a change to any of them without a rebuild would serve stale
/// wasm whose bytes disagree with the host.
#[test]
fn the_web_runtime_stamp_covers_the_transitive_dependency_closure() {
    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("xtask sits two levels under the repo root");
    let sources = crate::session::preflight::web_runtime_sources(repo_root);
    for required in [
        "Cargo.lock",
        "Cargo.toml",
        "clients/web-control",
        "crates/pilotage-input",
        "crates/pilotage-protocol",
    ] {
        assert!(
            sources.iter().any(|source| source == required),
            "the wasm stamp closure must cover {required}; got {sources:?}"
        );
    }
}
