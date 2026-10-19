#![allow(clippy::expect_used, clippy::panic)]

use super::{TREES, WRITERS, scan, spells_a_write};

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

#[test]
fn write_spellings_are_found_and_reads_are_not() {
    for line in [
        "let frame = encode_param_set(seq, name, value, 1, 1);",
        "self.send(ParamRequest::Set(GUID_OPTIONS, value));",
        "port.request(command::MSP_EEPROM_WRITE, vec![]).await?;",
    ] {
        assert!(spells_a_write(line), "{line}");
    }
    for line in [
        "self.send(ParamRequest::Read(MOT_THST_HOVER));",
        "encode_param_request_read(seq, name, 1, 1)",
        "port.send(command::MSP_SET_RAW_RC, payload).await?;",
    ] {
        assert!(!spells_a_write(line), "{line}");
    }
}

#[test]
fn a_planted_write_outside_the_writer_modules_fails_the_scan() {
    let root = std::env::temp_dir().join(format!("xtask-vehicle-writes-{}", std::process::id()));
    for tree in TREES {
        std::fs::create_dir_all(root.join(tree)).expect("create tree");
    }
    let planted = root.join("adapters/px4/src/uplink/shortcut.rs");
    let writer = root.join(WRITERS[0]);
    let test = root.join("adapters/px4/src/uplink/tests.rs");
    for file in [&planted, &writer, &test] {
        std::fs::create_dir_all(file.parent().expect("parent")).expect("create nested");
    }
    std::fs::write(
        &planted,
        "fn f() {}\nlet f = encode_param_set(0, \"X\", v, 1, 1);\n",
    )
    .expect("plant");
    std::fs::write(&writer, "let f = encode_param_set(0, \"X\", v, 1, 1);\n").expect("writer");
    std::fs::write(&test, "let f = encode_param_set(0, \"X\", v, 1, 1);\n").expect("test");

    let findings = scan(&root).expect("scan");
    let hits: Vec<_> = findings
        .iter()
        .map(|finding| (finding.path.clone(), finding.line))
        .collect();
    assert_eq!(hits, vec![(planted, 2)]);
    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn the_repository_writes_only_from_the_writer_modules() {
    let root = repo_root();
    for writer in WRITERS {
        assert!(
            root.join(writer).is_file(),
            "{writer} moved; update WRITERS"
        );
    }
    let findings = scan(&root).expect("scan");
    assert!(findings.is_empty(), "{findings:#?}");
}
