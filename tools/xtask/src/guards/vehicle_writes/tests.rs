#![allow(clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use super::{LINK, RULES, Rule, SITL_LAUNCH, scan, scan_rule};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
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
        assert!(LINK.spells_a_write(line), "{line}");
    }
    for line in [
        "self.send(ParamRequest::Read(MOT_THST_HOVER));",
        "encode_param_request_read(seq, name, 1, 1)",
        "port.send(command::MSP_SET_RAW_RC, payload).await?;",
        "// encode_param_set answers with a PARAM_VALUE",
    ] {
        assert!(!LINK.spells_a_write(line), "{line}");
    }
}

#[test]
fn sitl_start_up_overrides_are_found() {
    for line in [
        r#".map(|(name, value)| (format!("PX4_PARAM_{name}"), value)),"#,
        r#""--defaults".to_owned(),"#,
    ] {
        assert!(SITL_LAUNCH.spells_a_write(line), "{line}");
    }
    for line in [
        "//! a `PX4_PARAM_` override says so at start-up",
        "/// The single-session defaults (instance 0)",
    ] {
        assert!(!SITL_LAUNCH.spells_a_write(line), "{line}");
    }
}

/// Plants `line` in a non-writer module, the first writer module and a
/// test of a fresh root holding `rule`'s trees, and returns what the scan
/// names and the planted path.
fn scan_planted(
    rule: &Rule,
    tag: &str,
    planted: &str,
    line: &str,
) -> (Vec<(PathBuf, usize)>, PathBuf) {
    let root =
        std::env::temp_dir().join(format!("xtask-vehicle-writes-{tag}-{}", std::process::id()));
    for tree in rule.trees {
        std::fs::create_dir_all(root.join(tree)).expect("create tree");
    }
    let planted = root.join(planted);
    let writer = root.join(rule.writers[0]);
    let test = planted.with_file_name("tests.rs");
    for file in [&planted, &writer, &test] {
        std::fs::create_dir_all(file.parent().expect("parent")).expect("create nested");
    }
    std::fs::write(&planted, format!("fn f() {{}}\n{line}\n")).expect("plant");
    std::fs::write(&writer, format!("{line}\n")).expect("writer");
    std::fs::write(&test, format!("{line}\n")).expect("test");

    let findings = scan_rule(&root, rule).expect("scan");
    let hits = findings
        .iter()
        .map(|finding| (finding.path.clone(), finding.line))
        .collect();
    std::fs::remove_dir_all(&root).ok();
    (hits, planted)
}

#[test]
fn a_planted_link_write_outside_the_writer_modules_fails_the_scan() {
    let (hits, planted) = scan_planted(
        &LINK,
        "link",
        "adapters/px4/src/uplink/shortcut.rs",
        "let f = encode_param_set(0, \"X\", v, 1, 1);",
    );
    assert_eq!(hits, vec![(planted, 2)]);
}

#[test]
fn a_planted_sitl_override_outside_the_backend_modules_fails_the_scan() {
    let (hits, planted) = scan_planted(
        &SITL_LAUNCH,
        "sitl",
        "tools/xtask/src/backend/px4_gz.rs",
        r#"env.push((format!("PX4_PARAM_{name}"), value));"#,
    );
    assert_eq!(hits, vec![(planted, 2)]);
}

#[test]
fn the_repository_writes_only_from_the_writer_modules() {
    let root = repo_root();
    for rule in RULES {
        for writer in rule.writers {
            assert!(
                root.join(writer).is_file(),
                "{writer} moved; update the rule's writers"
            );
        }
    }
    let findings = scan(&root).expect("scan");
    assert!(findings.is_empty(), "{findings:#?}");
}
