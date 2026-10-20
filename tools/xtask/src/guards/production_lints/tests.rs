//! The production lint guard: the clippy plan, the unsafe-module scan,
//! and a real clippy run that refuses a local exemption.

#![allow(clippy::unwrap_used, clippy::panic)]

use super::{FfiCrate, clippy_invocations, findings_in};

mod ffi;

const BINDING: FfiCrate = FfiCrate {
    workspace_package: None,
    dir: "sim/binding",
    unsafe_module: "src/ffi.rs",
};

#[test]
fn without_ffi_crates_the_workspace_forbids_unsafe_code() {
    let invocations = clippy_invocations(&[]);
    assert_eq!(invocations.len(), 1);
    let args = invocations[0].join(" ");
    assert_eq!(
        args,
        "clippy --workspace --lib --bins --examples --no-deps -- -D warnings -F unsafe_code \
         -F clippy::unwrap_used -F clippy::expect_used -F clippy::panic"
    );
}

#[test]
fn an_ffi_crate_is_linted_apart_with_unsafe_code_denied() {
    let invocations = clippy_invocations(&[BINDING]);
    assert_eq!(invocations.len(), 2);
    let workspace = invocations[0].join(" ");
    assert!(workspace.contains("--workspace --lib --bins --examples --no-deps --"));
    assert!(workspace.contains("-F unsafe_code"));
    let binding = invocations[1].join(" ");
    assert!(binding.starts_with(
        "clippy --manifest-path sim/binding/Cargo.toml --lib --bins --examples --no-deps --"
    ));
    assert!(binding.contains("-D unsafe_code"));
    for lint in [
        "clippy::unwrap_used",
        "clippy::expect_used",
        "clippy::panic",
    ] {
        assert!(binding.contains(&format!("-F {lint}")), "{binding}");
        assert!(workspace.contains(&format!("-F {lint}")), "{workspace}");
    }
}

#[test]
fn mixed_ffi_plan_excludes_only_registered_workspace_members() {
    let member = FfiCrate {
        workspace_package: Some("binding-member"),
        dir: "adapters/binding-member",
        unsafe_module: "src/ffi.rs",
    };
    let invocations = clippy_invocations(&[BINDING, member]);
    assert_eq!(invocations.len(), 3);
    assert_eq!(
        invocations[0].join(" "),
        "clippy --workspace --lib --bins --examples --no-deps --exclude binding-member -- \
         -D warnings -F unsafe_code -F clippy::unwrap_used -F clippy::expect_used \
         -F clippy::panic"
    );
    for (args, manifest) in invocations[1..].iter().zip([
        "sim/binding/Cargo.toml",
        "adapters/binding-member/Cargo.toml",
    ]) {
        let args = args.join(" ");
        assert!(args.starts_with(&format!("clippy --manifest-path {manifest} ")));
        assert!(args.contains("-D unsafe_code"));
        assert!(args.contains("-F clippy::panic"));
    }
}

#[test]
fn unsafe_code_outside_the_unsafe_module_is_refused() {
    let text = "fn ok() {}\n// unsafe in a comment is fine\nfn bad() { unsafe { work() } }\n";
    let findings = findings_in("src/main.rs", text, false);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].line, 3);
    assert!(findings_in("src/main.rs", "#[allow(unsafe_code)]\nmod x;\n", false).len() == 1);
    assert!(findings_in("src/main.rs", "let unsafety = 1;\n", false).is_empty());
}

#[test]
fn every_unsafe_in_the_module_needs_a_safety_comment() {
    let justified = "\
// SAFETY: the handle is live for the borrow.
unsafe { call(handle) }
// SAFETY: the declarations match the shim's header.
// They take only plain data.
#[link(name = \"shim\")]
unsafe extern \"C\" {
    fn call(handle: *mut Handle);
}
let x = unsafe { read(p) }; // SAFETY: p is aligned and initialised.
";
    assert!(findings_in("src/ffi.rs", justified, true).is_empty());
    let unjustified = "\
// SAFETY: covers the first block only.
unsafe { one() }
let gap = 1;
unsafe { two() }
";
    let findings = findings_in("src/ffi.rs", unjustified, true);
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].line, 4);
}

/// The guard's clippy plan still refuses a local `allow` of a forbidden
/// lint: a real clippy run over a fixture crate.
#[test]
fn the_clippy_plan_refuses_a_local_expect_exemption() {
    let fixture = std::env::temp_dir().join(format!("plt_xt_lints_{}", std::process::id()));
    std::fs::remove_dir_all(&fixture).ok();
    std::fs::create_dir_all(fixture.join("src")).unwrap();
    std::fs::write(
        fixture.join("Cargo.toml"),
        "[package]\nname = \"production-lint-fixture\"\nversion = \"0.1.0\"\n\
         edition = \"2024\"\n\n[workspace]\n",
    )
    .unwrap();
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let run = |source: &str| {
        std::fs::write(fixture.join("src/lib.rs"), source).unwrap();
        let args = &clippy_invocations(&[])[0];
        std::process::Command::new(&cargo)
            .args(args)
            .current_dir(&fixture)
            .env("CARGO_TARGET_DIR", fixture.join("target"))
            .output()
            .unwrap()
    };
    let clean = run("pub fn first(v: &[u8]) -> Option<u8> {\n    v.first().copied()\n}\n");
    assert!(
        clean.status.success(),
        "{}",
        String::from_utf8_lossy(&clean.stderr)
    );
    let exempt = run(
        "#[allow(clippy::expect_used)]\npub fn first(v: &[u8]) -> u8 {\n    *v.first().expect(\"a value\")\n}\n",
    );
    assert!(!exempt.status.success());
    assert!(String::from_utf8_lossy(&exempt.stderr).contains("expect_used"));
    std::fs::remove_dir_all(&fixture).ok();
}
