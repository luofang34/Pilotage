//! A session's private web root serves the working-tree client and only
//! its own connect manifest.

#![allow(clippy::expect_used, clippy::panic)]

use super::{MANIFEST_NAME, private_web_root, viewer_source};

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
