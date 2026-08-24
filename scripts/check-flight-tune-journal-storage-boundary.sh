#!/usr/bin/env bash
# Keep core journal file-system access behind the durable-storage crate.
set -euo pipefail

repo_root="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
journal_file="$repo_root/tools/flight-tune/src/journal.rs"
journal_root="$repo_root/tools/flight-tune/src/journal"

if [ ! -f "$journal_file" ] || [ ! -d "$journal_root" ]; then
    echo "the flight-tune journal source is missing" >&2
    exit 1
fi

sources=("$journal_file")
if ! source_list="$(
    find "$journal_root" \
        -type f \
        -name '*.rs' \
        ! -name 'tests.rs' \
        ! -path '*/tests/*' \
        -print
)"; then
    echo "the flight-tune journal source scan failed" >&2
    exit 1
fi
while IFS= read -r source; do
    if [ -n "$source" ]; then
        sources+=("$source")
    fi
done <<< "$source_list"

forbidden='(^|[^[:alnum:]_])(fs|File|OpenOptions|NamedTempFile|TempDir|rustix|libc|nix|cap_std|camino|tempfile|walkdir|Command|Stdio)([^[:alnum:]_]|$)|std[[:space:]]*::[[:space:]]*process'

if matches="$(grep -En "$forbidden" "${sources[@]}" 2>&1)"; then
    printf '%s\n' "$matches"
    echo "FORBIDDEN: the core journal bypasses pilotage-durable-storage" >&2
    exit 1
else
    status=$?
    if [ "$status" -ne 1 ]; then
        printf '%s\n' "$matches" >&2
        echo "the flight-tune journal storage scan failed" >&2
        exit "$status"
    fi
fi

echo "flight-tune journal storage boundary: OK"
