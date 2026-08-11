#!/usr/bin/env bash
# The allowlist makes each production compound addition an explicit review item.
# This prevents a monotonic counter from using debug-panicking arithmetic.
set -euo pipefail

root_dir="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
allowlist="${2:-$root_dir/scripts/monotonic-counter-compound-addition-allowlist.tsv}"
actual=$(mktemp)
trap 'rm -f "$actual"' EXIT

collect_rs_files() {
    git -C "$root_dir" ls-files --cached --others --exclude-standard -- '*.rs'
}

collect_compound_additions() {
    local file
    while IFS= read -r file; do
        case "$file" in
            */generated/*|*/tests/*|*/tests.rs|*/test.rs|*/*_tests.rs) continue ;;
        esac
        awk -v path="$file" '
            index($0, "+=") {
                line = $0
                sub(/^[[:space:]]*/, "", line)
                if (line !~ /^\/\//) {
                    print path "\t" line
                }
            }
        ' "$root_dir/$file"
    done < <(collect_rs_files)
}

collect_compound_additions | LC_ALL=C sort > "$actual"
if ! diff -u "$allowlist" "$actual"; then
    echo "FORBIDDEN: production compound addition differs from the reviewed allowlist" >&2
    echo "Use wrapping_add for a monotonic counter." >&2
    exit 1
fi

echo "monotonic counter arithmetic: OK"
