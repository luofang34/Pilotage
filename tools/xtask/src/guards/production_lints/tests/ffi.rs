//! Exercise the exact FFI boundary through the source scan and real Clippy.

use std::path::{Path, PathBuf};
use std::process::Output;

use super::super::{FFI_CRATES, FfiCrate, clippy_invocations, unsafe_findings};
use super::BINDING;

const USB: FfiCrate = FfiCrate {
    workspace_package: Some("pilotage-dji-usb"),
    dir: "adapters/dji-usb",
    feature: None,
    unsafe_module: "src/ffi.rs",
};

const FFI_SOURCE: &str = "\
// SAFETY: Only this module dereferences the fixture's live stack value.
#![allow(unsafe_code)]
pub fn value() -> u8 {
    let value = 1_u8;
    // SAFETY: The pointer is aligned and valid for this synchronous read.
    unsafe { core::ptr::read(&value) }
}
";

#[test]
fn dji_usb_registers_only_its_ffi_source_file() {
    assert!(FFI_CRATES.contains(&USB));
    assert_eq!(
        FFI_CRATES
            .iter()
            .filter(|entry| entry.workspace_package == USB.workspace_package)
            .count(),
        1
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let findings = unsafe_findings(&root, &[USB]).unwrap();
    assert!(findings.is_empty(), "{findings:?}");
}

#[test]
fn an_optional_binding_is_linted_with_its_feature_on() {
    let sidecar = FfiCrate {
        workspace_package: None,
        dir: "sim/jsbsim-sidecar",
        feature: Some("cpp"),
        unsafe_module: "src/engine/ffi.rs",
    };
    assert!(FFI_CRATES.contains(&sidecar));
    let plan = clippy_invocations(&[sidecar]);
    assert!(plan[1].join(" ").contains("--features cpp"));
}

#[test]
fn ffi_plan_accepts_its_boundary_but_refuses_other_unsafe_and_panic_exemptions() {
    let root = fixture();
    let usb_source = root.join(USB.dir).join("src/lib.rs");
    let plan = clippy_invocations(&[USB, BINDING]);
    assert!(unsafe_findings(&root, &[USB, BINDING]).unwrap().is_empty());
    for args in &plan {
        let output = run_clippy(&root, args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    std::fs::write(
        &usb_source,
        "mod ffi;\npub use ffi::value;\n\
         pub fn outside() -> u8 { let value = 0; unsafe { core::ptr::read(&value) } }\n",
    )
    .unwrap();
    let findings = unsafe_findings(&root, &[USB]).unwrap();
    assert_eq!(findings.len(), 1);
    assert!(findings[0].path.ends_with("src/lib.rs"));
    let output = run_clippy(&root, &plan[1]);
    assert!(!output.status.success());
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(
        diagnostic.contains("unsafe-code") || diagnostic.contains("unsafe_code"),
        "{diagnostic}"
    );

    std::fs::write(
        &usb_source,
        "mod ffi;\npub use ffi::value;\n\
         #[allow(clippy::expect_used)]\n\
         pub fn first(values: &[u8]) -> u8 { *values.first().expect(\"value\") }\n",
    )
    .unwrap();
    let output = run_clippy(&root, &plan[1]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("expect_used"));

    std::fs::write(
        &usb_source,
        "#![allow(unsafe_code)]\nmod ffi;\npub use ffi::value;\n",
    )
    .unwrap();
    let findings = unsafe_findings(&root, &[USB]).unwrap();
    assert_eq!(findings.len(), 1);
    assert!(findings[0].path.ends_with("src/lib.rs"));
    std::fs::write(&usb_source, "mod ffi;\npub use ffi::value;\n").unwrap();
    std::fs::write(
        root.join("consumer/src/lib.rs"),
        "#![allow(unsafe_code)]\n\
         pub fn value() -> u8 { let value = 0; unsafe { core::ptr::read(&value) } }\n",
    )
    .unwrap();
    let output = run_clippy(&root, &plan[0]);
    assert!(!output.status.success());
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(
        diagnostic.contains("unsafe-code") || diagnostic.contains("unsafe_code"),
        "{diagnostic}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(test)]
fn fixture() -> PathBuf {
    let root = std::env::temp_dir().join(format!("plt_xt_ffi_lints_{}", std::process::id()));
    std::fs::remove_dir_all(&root).ok();
    let source = root.join(USB.dir).join("src");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir_all(root.join("consumer/src")).unwrap();
    standalone_fixture(&root);
    std::fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"adapters/dji-usb\", \"consumer\"]\nexclude = [\"sim/binding\"]\nresolver = \"2\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join(USB.dir).join("Cargo.toml"),
        "[package]\nname = \"pilotage-dji-usb\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    std::fs::write(source.join("lib.rs"), "mod ffi;\npub use ffi::value;\n").unwrap();
    std::fs::write(source.join("ffi.rs"), FFI_SOURCE).unwrap();
    std::fs::write(
        root.join("consumer/Cargo.toml"),
        "[package]\nname = \"consumer\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\
         [dependencies]\npilotage-dji-usb = { path = \"../adapters/dji-usb\" }\n",
    )
    .unwrap();
    std::fs::write(
        root.join("consumer/src/lib.rs"),
        "pub fn value() -> u8 { pilotage_dji_usb::value() }\n",
    )
    .unwrap();
    root
}

#[cfg(test)]
fn standalone_fixture(root: &Path) {
    let source = root.join(BINDING.dir).join("src");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(
        root.join(BINDING.dir).join("Cargo.toml"),
        "[package]\nname = \"binding\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[workspace]\n",
    )
    .unwrap();
    std::fs::write(source.join("lib.rs"), "mod ffi;\npub use ffi::value;\n").unwrap();
    std::fs::write(source.join("ffi.rs"), FFI_SOURCE).unwrap();
}

#[cfg(test)]
fn run_clippy(root: &Path, args: &[String]) -> Output {
    std::process::Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned()))
        .args(args)
        .current_dir(root)
        .env("CARGO_TARGET_DIR", root.join("target"))
        .output()
        .unwrap()
}
