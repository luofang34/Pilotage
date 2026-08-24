#!/usr/bin/env bash
# Prove that the journal storage boundary rejects direct file-system access.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
guard="$repo_root/scripts/check-flight-tune-journal-storage-boundary.sh"
fixture="$(mktemp -d "${TMPDIR:-/tmp}/pilotage-journal-boundary.XXXXXX")"
trap 'rm -rf "$fixture"' EXIT

journal_root="$fixture/tools/flight-tune/src/journal"
mkdir -p "$journal_root"
printf '%s\n' 'pub struct Journal;' > "$fixture/tools/flight-tune/src/journal.rs"
printf '%s\n' 'use pilotage_durable_storage::DurableStore;' > "$journal_root/storage.rs"

bash "$guard" "$fixture" >/dev/null

assert_rejected() {
    local source="$1"
    printf '%s\n' "$source" > "$journal_root/storage.rs"
    if bash "$guard" "$fixture" >/dev/null 2>&1; then
        echo "the journal storage boundary accepted: $source" >&2
        exit 1
    fi
}

assert_rejected 'use std::{fs as durable};'
assert_rejected 'use rustix::fs::renameat;'
assert_rejected 'use std::process::Command;'

printf '%s\n' 'use pilotage_durable_storage::DurableStore;' > "$journal_root/storage.rs"
printf '%s\n' 'use std::fs;' > "$journal_root/tests.rs"
bash "$guard" "$fixture" >/dev/null

mkdir -p "$fixture/bin"
printf '%s\n' '#!/bin/sh' 'exit 2' > "$fixture/bin/rg"
chmod +x "$fixture/bin/rg"
if PATH="$fixture/bin:$PATH" bash "$guard" "$fixture" >/dev/null 2>&1; then
    echo "the journal storage boundary ignored a scanner failure" >&2
    exit 1
fi

echo "flight-tune journal storage boundary self-test: OK"
