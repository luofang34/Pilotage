#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
check="$repo_root/scripts/check-control-portability.sh"
bash "$check" "$repo_root" >/dev/null

fixture="$(mktemp -d "${TMPDIR:-/tmp}/pilotage-control-portability.XXXXXX")"
trap 'rm -rf "$fixture"' EXIT
mkdir -p "$fixture/crates/pilotage-control" "$fixture/crates/pilotage-input"
write_clean() {
    printf '[dependencies]\npilotage-input = { workspace = true }\nserde = { workspace = true }\n' \
        >"$fixture/crates/pilotage-control/Cargo.toml"
    printf '[dependencies]\nthiserror = { workspace = true }\n' \
        >"$fixture/crates/pilotage-input/Cargo.toml"
}

write_clean
bash "$check" "$fixture" >/dev/null

expect_failure() {
    local why="$1"
    if bash "$check" "$fixture" >/dev/null 2>&1; then
        echo "check-control-portability accepted $why" >&2
        exit 1
    fi
    write_clean
}

printf '[target.wasm32-unknown-unknown.dependencies]\nwasm-bindgen = { workspace = true }\n' \
    >>"$fixture/crates/pilotage-control/Cargo.toml"
expect_failure "a direct wasm-bindgen dependency"

printf 'web-sys = "0.3"\n' >>"$fixture/crates/pilotage-input/Cargo.toml"
expect_failure "a browser binding in a workspace dependency"

rm "$fixture/crates/pilotage-input/Cargo.toml"
expect_failure "a workspace dependency it cannot read"
