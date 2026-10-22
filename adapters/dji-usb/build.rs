//! Compile the macOS IOKit binding for the current target.

use std::env;
use std::error::Error;
use std::path::PathBuf;
use std::process::Command;

fn main() -> Result<(), Box<dyn Error>> {
    for file in [
        "native/logiclink.h",
        "native/discovery.h",
        "native/discovery.m",
        "native/discovery_tests.m",
        "native/access.h",
        "native/access.m",
        "native/access_guard.m",
        "native/access_discovery.m",
        "native/watch.h",
        "native/watch.m",
        "native/profile.m",
        "native/interface.m",
        "native/tests.m",
    ] {
        // Cargo build directives are protocol output, not diagnostics.
        use std::io::Write;
        writeln!(std::io::stdout(), "cargo:rerun-if-changed={file}")?;
    }
    if env::var("CARGO_CFG_TARGET_OS")? != "macos" {
        return Ok(());
    }
    let architecture = match env::var("CARGO_CFG_TARGET_ARCH")?.as_str() {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => return Err(format!("unsupported macOS architecture: {other}").into()),
    };
    let out = PathBuf::from(env::var_os("OUT_DIR").ok_or("OUT_DIR is absent")?);
    let mut objects = Vec::new();
    for name in [
        "profile",
        "interface",
        "tests",
        "discovery",
        "discovery_tests",
        "watch",
        "access",
        "access_guard",
        "access_discovery",
    ] {
        let object = out.join(format!("{name}.o"));
        let status = Command::new("xcrun")
            .args(["clang", "-fobjc-arc", "-Wall", "-Wextra", "-Werror", "-c"])
            .args(["-arch", architecture])
            .arg(format!("native/{name}.m"))
            .arg("-o")
            .arg(&object)
            .status()?;
        if !status.success() {
            return Err(format!("native {name} compilation failed: {status}").into());
        }
        objects.push(object);
    }
    let status = Command::new("xcrun")
        .args(["ar", "crs"])
        .arg(out.join("libpilotage_dji_usb.a"))
        .args(objects)
        .status()?;
    if !status.success() {
        return Err(format!("native archive failed: {status}").into());
    }
    use std::io::Write;
    let mut output = std::io::stdout().lock();
    writeln!(output, "cargo:rustc-link-search=native={}", out.display())?;
    writeln!(output, "cargo:rustc-link-lib=static=pilotage_dji_usb")?;
    writeln!(output, "cargo:rustc-link-lib=framework=Foundation")?;
    writeln!(output, "cargo:rustc-link-lib=framework=IOKit")?;
    Ok(())
}
