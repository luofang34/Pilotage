//! The single sanctioned place this binary writes to stdout.
//!
//! `disallowed_macros` (ADR-0015) bans bare `println!` everywhere else
//! in the workspace so library crates never grow CLI side effects; this
//! module is the CLI-product-output exception the lint rule anticipates
//! (the admission summary, digest verdict, and written-frame paths are
//! this tool's deliverable, not a debug trace).

/// Writes one line of user-facing CLI output to stdout.
// WHY: the conformance verdict and digest are the product of this tool,
// so the workspace-wide println ban is waived here and nowhere else in
// this binary.
#[allow(clippy::disallowed_macros)]
pub fn print_line(line: &str) {
    println!("{line}");
}
