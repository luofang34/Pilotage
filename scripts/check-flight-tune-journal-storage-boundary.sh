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
while IFS= read -r source; do
    sources+=("$source")
done < <(
    rg --files "$journal_root" \
        -g '*.rs' \
        -g '!**/tests.rs' \
        -g '!**/tests/**'
)

forbidden='(^|[^[:alnum:]_])(fs|File|OpenOptions|NamedTempFile|TempDir|rustix|libc|nix|cap_std|camino|tempfile|walkdir|Command|Stdio)([^[:alnum:]_]|$)|std[[:space:]]*::[[:space:]]*process'

if rg -n --pcre2 "$forbidden" "${sources[@]}"; then
    echo "FORBIDDEN: the core journal bypasses pilotage-durable-storage" >&2
    exit 1
fi

echo "flight-tune journal storage boundary: OK"
