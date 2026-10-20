//! A session's private web root serves the working-tree client and only
//! its own connect manifest.

#![allow(clippy::expect_used, clippy::panic)]

use super::{MANIFEST_NAME, private_web_root, viewer_source};

#[test]
fn host_stage_binds_locally_unless_lan_has_an_explicit_join_code_file() {
    use crate::backend::{SessionContext, backend_for};

    let mut ctx = SessionContext::for_test(std::env::temp_dir());
    let backend = backend_for("ardupilot-sitl").expect("known adapter");
    let local = super::host_stage(&ctx, backend.as_ref()).expect("local stage");
    assert_eq!(
        local.spec.args,
        [
            "--port",
            "4433",
            "--adapter",
            "ardupilot",
            "--bind",
            "127.0.0.1"
        ]
    );
    ctx.lan = true;
    assert!(super::host_stage(&ctx, backend.as_ref()).is_err());
    ctx.join_token_file = Some(std::path::PathBuf::from("relative-code"));
    assert!(super::host_stage(&ctx, backend.as_ref()).is_err());
    ctx.join_token_file = Some(std::env::temp_dir().join("operator-code"));
    let lan = super::host_stage(&ctx, backend.as_ref()).expect("explicit remote stage");
    assert_eq!(
        lan.spec.args,
        [
            "--port".to_owned(),
            "4433".to_owned(),
            "--adapter".to_owned(),
            "ardupilot".to_owned(),
            "--bind".to_owned(),
            "0.0.0.0".to_owned(),
            "--join-token-file".to_owned(),
            std::env::temp_dir()
                .join("operator-code")
                .display()
                .to_string(),
        ]
    );
}

#[test]
fn join_code_bytes_never_enter_process_arguments_or_public_connect_facts() {
    use crate::backend::{SessionContext, backend_for};
    use crate::readiness::{session_manifest, viewer_url};

    let token_path = std::env::temp_dir().join(format!("plt_join_code_{}", std::process::id()));
    let secret = "b7".repeat(32);
    std::fs::write(&token_path, &secret).expect("private code file");
    let mut ctx = SessionContext::for_test(std::env::temp_dir());
    ctx.lan = true;
    ctx.join_token_file = Some(token_path.clone());
    let backend = backend_for("ardupilot-sitl").expect("known adapter");
    let stage = super::host_stage(&ctx, backend.as_ref()).expect("LAN stage");
    assert!(!format!("{:?}", stage.spec).contains(&secret));
    for published in [
        session_manifest("pilotage.local", None, 4433, &"cd".repeat(32)),
        viewer_url("pilotage.local", 8080, 4433, &"cd".repeat(32)),
    ] {
        assert!(!published.contains(&secret));
        assert!(!published.contains(&token_path.display().to_string()));
        assert!(!published.contains("join"));
    }
    std::fs::remove_file(&token_path).expect("remove private code fixture");
}

#[test]
fn a_private_web_root_serves_the_tree_but_never_the_shared_manifest() {
    let base = std::env::temp_dir().join(format!("plt_xt_webroot_{}", std::process::id()));
    std::fs::remove_dir_all(&base).ok();
    let source = viewer_source(&base);
    std::fs::create_dir_all(source.join("vendor")).expect("viewer tree");
    std::fs::write(source.join("index.html"), "<title>viewer</title>").expect("entry");
    std::fs::write(source.join("vendor/map.js"), "map").expect("nested asset");
    std::fs::write(source.join(MANIFEST_NAME), "{\"other\":\"session\"}").expect("manifest");
    let run_dir = base.join("target/xtask-fleet/px4-gz-1");

    let root = private_web_root(&base, &run_dir).expect("web root builds");
    assert_eq!(
        std::fs::read_to_string(root.join("index.html")).expect("entry served"),
        "<title>viewer</title>"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("vendor/map.js")).expect("nested asset served"),
        "map"
    );
    assert!(
        !root.join(MANIFEST_NAME).exists(),
        "another session's manifest must not be served here"
    );

    // A relaunch rebuilds the root over the previous one, and writing this
    // session's manifest leaves the shared one untouched.
    let root = private_web_root(&base, &run_dir).expect("web root rebuilds");
    std::fs::write(root.join(MANIFEST_NAME), "{\"this\":\"session\"}").expect("own manifest");
    assert_eq!(
        std::fs::read_to_string(source.join(MANIFEST_NAME)).expect("shared manifest"),
        "{\"other\":\"session\"}"
    );
    std::fs::remove_dir_all(&base).ok();
}
