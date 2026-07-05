//! The single sanctioned place this binary writes to stdout.
//!
//! `disallowed_macros` (ADR-0015) bans bare `println!`/`eprintln!` everywhere
//! else in the workspace so that library crates never accidentally grow CLI
//! side effects; this module is the CLI-product-output exception the lint
//! rule anticipates (the run summary is this tool's actual deliverable, not
//! a debug trace). Diagnostics still go through `tracing`.

/// Writes one line of user-facing CLI output to stdout.
// WHY: this is the tool's actual product (the end-of-run measurement
// summary), not a debug trace, so the workspace-wide println ban is waived
// here and nowhere else in this binary.
#[allow(clippy::disallowed_macros)]
pub fn print_line(line: &str) {
    println!("{line}");
}
