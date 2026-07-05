# Contributing to Pilotage

## Toolchain setup

1. Install the Rust toolchain pinned by `rust-toolchain.toml` (stable channel);
   `rustup` will pick it up automatically when you run any `cargo` command in
   this repository.
2. Install `protoc` (needed by `prost-build` for crates that compile
   `schemas/*.proto`).
3. Install `shellcheck` (used to lint `scripts/*.sh`).
4. On Linux, install `libudev-dev` (required to build `hidapi`).
5. If `schemas/` contains `.proto` files, install `buf` to run `buf lint`
   locally before pushing.

## Local gate commands

Run these in order before opening a PR — they are the same gates CI runs in
`.github/workflows/ci.yml`, and all of them are blocking:

```bash
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
RUSTDOCFLAGS='-D missing_docs -D rustdoc::broken_intra_doc_links' cargo doc --no-deps
cargo build --release
bash scripts/check-structure.sh
```

If `schemas/` contains `.proto` files, also run `buf lint` from the repo
root.

## PR discipline

- One issue per PR. Break large refactors into independently revertible
  steps.
- Every PR lands the fix **and** the guardrail that prevents its regression
  (a test, a lint, or a CI script change) in the same PR. A fix without a
  guardrail is temporary.
- Do not skip hooks, force-push shared branches, or bypass the gates above to
  land a PR faster; if a gate is wrong, fix the gate in its own PR first.

## Workspace conventions

See `docs/adr/0002-cargo-workspace-portable-sans-io-core.md` and
`docs/adr/0015-workspace-quality-gates.md` for the architectural and lint
rules this repository enforces mechanically (sans-IO core crates, structure
limits, error-handling conventions, and the CI pipeline order).
