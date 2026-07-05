//! The single sanctioned place this binary writes to stdout.
//!
//! `disallowed_macros` (ADR-0015) bans bare `println!`/`eprintln!` everywhere
//! else in the workspace so that library crates never accidentally grow CLI
//! side effects; this module is the CLI-product-output exception the lint
//! rule anticipates, not a diagnostic channel (diagnostics still go through
//! `tracing` if this binary ever needs them).

/// Writes one line of user-facing CLI output to stdout.
// WHY: this is the tool's actual product (human/machine-readable probe
// output), not a debug trace, so the workspace-wide println ban is waived
// here and nowhere else in this binary.
#[allow(clippy::disallowed_macros)]
pub fn print_line(line: &str) {
    println!("{line}");
}
