#!/bin/sh
# Reject panic-capable Rust operations and unsafe code in production targets.
set -eu

repo_root=${1:-"$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)"}

cd "$repo_root"
cargo clippy --workspace --lib --bins --examples -- \
    -D warnings \
    -F unsafe_code \
    -F clippy::unwrap_used \
    -F clippy::expect_used \
    -F clippy::panic
