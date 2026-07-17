//! The single sanctioned place this binary writes to stdout.
//!
//! `disallowed_macros` (ADR-0015) bans bare `println!` everywhere else in
//! the workspace so library crates never grow CLI side effects; this
//! module is the CLI-product-output exception the lint rule anticipates
//! (launch progress and the ready URL are this tool's deliverable, not a
//! debug trace). Diagnostics still go through `tracing`.

/// Writes one line of user-facing CLI output to stdout.
// WHY: launch progress and the session-ready URL are the product of this
// tool, so the workspace-wide println ban is waived here and nowhere else
// in this binary.
#[allow(clippy::disallowed_macros)]
pub fn print_line(line: &str) {
    println!("{line}");
}
