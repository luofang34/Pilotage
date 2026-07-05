# ADR-0015: Workspace-enforced quality gates

- Status: Accepted
- Date: 2026-07-05

## Context

Authority and timing defects in this domain are safety-relevant: a panic in the
session host is a dropped vehicle link, and a swallowed error in the authority engine
is an ownership dispute. Conventions that live only in review culture decay; the
repository must make its rules mechanical, and every fix must land together with the
guardrail that prevents its regression.

## Decision

### Workspace lint policy (`[workspace.lints]`, inherited by every crate)

- `unsafe_code = "forbid"`.
- `clippy::unwrap_used`, `clippy::expect_used`, `clippy::panic` = deny. Tests may
  `#[allow(clippy::expect_used, clippy::panic)]`.
- `let_underscore_must_use` and `let_underscore_future` = deny; intentional discard
  is written `.ok()`.
- `clippy::await_holding_lock = "deny"`.
- `clippy::disallowed_types` bans `anyhow::Error` in library crates;
  `clippy::disallowed_macros` bans `println!`/`eprintln!` outside explicit CLI
  output modules — diagnostics use `tracing`.

### Error handling

- Library crates use `thiserror` typed enums; `anyhow` appears only in binary
  `main`. Error context is never discarded (`#[source]`/`#[from]`), each variant
  carries what its message needs, and there is no `From<anyhow::Error>` back door
  into typed errors.
- No `process::exit()`; errors propagate to `main`.

### Structure limits (enforced by a CI script, not convention)

- ≤ 500 lines per `.rs` file; ≤ 80 lines per function; ≤ 30 fields per struct and
  variants per enum; `lib.rs` ≤ 100 lines, re-exports and module declarations only,
  opening with a crate-level `//!` doc comment.
- No `mod.rs` (use `foo.rs` + `foo/`); no `utils.rs`/`helpers.rs`/`common.rs` —
  modules are named by domain.

### Idioms

- Monotonic counters (IDs, generations, epochs) advance with `wrapping_add(1)`.
- No module-level mutable state with I/O side effects; configuration flows through a
  central config struct, not scattered `env::var` reads.
- Blocking functions carry a `_blocking` suffix; possibly-stale reads carry
  `_cached`.

### CI pipeline (in order, all blocking)

```text
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo doc  (RUSTDOCFLAGS: -D missing_docs -D rustdoc::broken_intra_doc_links)
cargo build --release
structure-limits script
buf lint && buf breaking   (once schemas/ is populated)
```

### Process

- Every bug-fix PR lands its regression guardrail (test, lint, or CI script) in the
  same PR.
- Tests synchronize on events (`Notify`, channels, completion handles), never
  sleep-and-retry; test servers bind `127.0.0.1:0`.

## Consequences

- The first workspace commit must carry the lint table and CI workflow — gates
  precede the code they gate.
- Some prototyping friction is accepted deliberately; spike code that needs `unwrap`
  lives in tests or examples, not in library crates.
- Structure limits force early module splits, which keeps the sans-IO core
  reviewable as it grows.
