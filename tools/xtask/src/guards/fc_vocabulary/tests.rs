#![allow(clippy::expect_used, clippy::panic)]

use super::{CLIENT_FLAT_DIRS, CLIENT_TREES, names_fc_vocabulary, scan};

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

#[test]
fn flight_controller_names_are_found() {
    for line in [
        "const MAV_RESULT_IN_PROGRESS: u32 = 5;",
        "// GIMBAL_DEVICE_FLAGS bit 32 is yaw lock",
        "use pilotage_mavlink::codec;",
        "the raw MAVLink result",
    ] {
        assert!(names_fc_vocabulary(line), "{line}");
    }
}

#[test]
fn typed_protocol_names_are_not_findings() {
    for line in [
        "CommandOutcome::TemporarilyRejected",
        "last_command_outcome",
        "GimbalAttitude { yaw_frame }",
        "max_avionics_rate",
    ] {
        assert!(!names_fc_vocabulary(line), "{line}");
    }
}

#[test]
fn a_planted_constant_in_a_client_tree_fails_the_scan() {
    let root = std::env::temp_dir().join(format!("xtask-fc-vocabulary-{}", std::process::id()));
    for dir in CLIENT_TREES.iter().chain(CLIENT_FLAT_DIRS) {
        std::fs::create_dir_all(root.join(dir)).expect("create tree");
    }
    let planted = root.join("crates/pilotage-control/src/deep/telegraph.rs");
    std::fs::create_dir_all(planted.parent().expect("parent")).expect("create nested");
    std::fs::write(&planted, "fn f() {}\nconst MAV_RESULT_DENIED: u32 = 2;\n").expect("plant");
    std::fs::write(
        root.join("clients/web/wire.js"),
        "// last_command_result is MAV_RESULT\n",
    )
    .expect("plant js");

    let findings = scan(&root).expect("scan");
    let hits: Vec<_> = findings
        .iter()
        .map(|finding| (finding.path.file_name().map(|n| n.to_owned()), finding.line))
        .collect();
    assert_eq!(
        hits,
        vec![
            (Some("wire.js".into()), 1),
            (Some("telegraph.rs".into()), 2),
        ]
    );
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn a_missing_client_tree_is_an_error_not_a_clean_result() {
    let root =
        std::env::temp_dir().join(format!("xtask-fc-vocabulary-empty-{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("create root");
    assert!(scan(&root).is_err());
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn the_repository_clients_carry_no_flight_controller_vocabulary() {
    let findings = scan(&repo_root()).expect("scan");
    assert!(findings.is_empty(), "{findings:#?}");
}
