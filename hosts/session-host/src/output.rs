//! The single sanctioned place this binary writes to stdout.
//!
//! `disallowed_macros` (ADR-0015) bans bare `println!`/`eprintln!` everywhere
//! else in the workspace so library crates never accidentally grow CLI side
//! effects. This module is the CLI-product-output exception the lint rule
//! anticipates: the `LISTENING <port> <cert-hash-hex>` line is a
//! machine-readable startup contract a wrapping process parses, not a
//! diagnostic trace (diagnostics go through `tracing` everywhere else in this
//! binary).

/// Writes one line of machine-readable startup output to stdout.
// WHY: this is the host's actual product contract for scripts/tests waiting
// on startup, not a debug trace, so the workspace-wide println ban is waived
// here and nowhere else in this binary.
#[allow(clippy::disallowed_macros)]
pub fn print_line(line: &str) {
    println!("{line}");
}
