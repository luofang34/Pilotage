#!/usr/bin/env bash
# Enforces the structural limits from ADR-0015 that are not expressible as
# rustc/clippy lints:
#   - no mod.rs files
#   - no utils.rs / helpers.rs / common.rs files
#   - no tracked .rs file over 500 lines (excluding target/ and any
#     /generated/ path)
#   - no lib.rs over 100 lines
#   - no function body over 80 lines
#
# The function-length check is an AWK brace-depth heuristic: it counts lines
# between a `fn` header and the point where brace depth returns to the level
# it had when the function opened. It does not parse Rust; it can be
# confused by braces inside string literals, char literals, or comments.
# Treat violations it reports as a strong signal, not ground truth.
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root_dir"

status=0

is_excluded_path() {
    case "$1" in
        */target/*|target/*) return 0 ;;
        */generated/*) return 0 ;;
        *) return 1 ;;
    esac
}

collect_rs_files() {
    find . \
        -type d \( -name target -o -name generated \) -prune -o \
        -type f -name '*.rs' -print
}

check_forbidden_filenames() {
    local file base
    while IFS= read -r file; do
        is_excluded_path "$file" && continue
        base="$(basename "$file")"
        case "$base" in
            mod.rs)
                echo "FORBIDDEN: $file (no mod.rs; use foo.rs + foo/)" >&2
                status=1
                ;;
            utils.rs|helpers.rs|common.rs)
                echo "FORBIDDEN: $file (no generic utils/helpers/common modules)" >&2
                status=1
                ;;
        esac
    done < <(collect_rs_files)
}

check_file_length() {
    local file base lines limit
    while IFS= read -r file; do
        is_excluded_path "$file" && continue
        base="$(basename "$file")"
        lines="$(wc -l < "$file" | tr -d ' ')"
        limit=500
        if [ "$base" = "lib.rs" ]; then
            limit=100
        fi
        if [ "$lines" -gt "$limit" ]; then
            echo "FORBIDDEN: $file has $lines lines (limit $limit)" >&2
            status=1
        fi
    done < <(collect_rs_files)
}

check_function_length() {
    local file
    while IFS= read -r file; do
        is_excluded_path "$file" && continue
        awk -v fname="$file" '
            function report(len, start) {
                if (len > 80) {
                    printf "FORBIDDEN: %s:%d function body has %d lines (limit 80)\n", fname, start, len > "/dev/stderr"
                    bad = 1
                }
            }
            BEGIN { depth = 0; in_fn = 0; fn_depth = 0; fn_start = 0; body_lines = 0; bad = 0 }
            {
                line = $0
                if (!in_fn && line ~ /\bfn[ \t]+[A-Za-z_][A-Za-z0-9_]*[ \t]*(<[^>]*>)?[ \t]*\(/) {
                    in_fn = 1
                    fn_depth = depth
                    fn_start = NR
                    body_lines = 0
                    has_opened = 0
                }
                if (in_fn) {
                    body_lines++
                }
                n_open = gsub(/\{/, "{", line)
                n_close = gsub(/\}/, "}", line)
                depth += n_open
                if (in_fn && n_open > 0) {
                    has_opened = 1
                }
                depth -= n_close
                if (in_fn && has_opened && depth <= fn_depth) {
                    report(body_lines, fn_start)
                    in_fn = 0
                }
            }
            END { exit bad }
        ' "$file" || status=1
    done < <(collect_rs_files)
}

check_forbidden_filenames
check_file_length
check_function_length

if [ "$status" -ne 0 ]; then
    echo "check-structure: FAILED" >&2
    exit 1
fi

echo "check-structure: OK"
