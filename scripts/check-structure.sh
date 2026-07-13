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
function_baseline="scripts/structure-function-baseline.tsv"

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
        awk -v fname="$file" -v baseline="$function_baseline" '
            function report(name, len, start, key, limit) {
                key = fname SUBSEP name
                seen[key] = 1
                if (key in allowed) {
                    limit = allowed[key]
                    if (len != limit) {
                        printf "FORBIDDEN: %s:%d function %s has %d lines; baseline requires exactly %d\n", fname, start, name, len, limit > "/dev/stderr"
                        bad = 1
                    }
                } else if (len > 80) {
                    printf "FORBIDDEN: %s:%d function body has %d lines (limit 80)\n", fname, start, len > "/dev/stderr"
                    bad = 1
                }
            }
            BEGIN {
                while ((getline entry < baseline) > 0) {
                    if (entry ~ /^[ \t]*#/ || entry ~ /^[ \t]*$/) {
                        continue
                    }
                    split(entry, fields, "\t")
                    key = fields[1] SUBSEP fields[2]
                    allowed[key] = fields[3] + 0
                    allowed_file[key] = fields[1]
                }
                close(baseline)
                depth = 0
                in_fn = 0
                fn_depth = 0
                fn_start = 0
                body_lines = 0
                bad = 0
            }
            {
                line = $0
                if (!in_fn && line ~ /(^|[^[:alnum:]_])fn[ \t]+[A-Za-z_][A-Za-z0-9_]*[ \t]*(<[^>]*>)?[ \t]*\(/) {
                    match(line, /fn[ \t]+[A-Za-z_][A-Za-z0-9_]*/)
                    fn_name = substr(line, RSTART, RLENGTH)
                    sub(/^fn[ \t]+/, "", fn_name)
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
                    report(fn_name, body_lines, fn_start)
                    in_fn = 0
                }
            }
            END {
                for (key in allowed) {
                    if (allowed_file[key] == fname && !(key in seen)) {
                        split(key, parts, SUBSEP)
                        printf "FORBIDDEN: baseline function %s in %s was not found\n", parts[2], fname > "/dev/stderr"
                        bad = 1
                    }
                }
                exit bad
            }
        ' "$file" || status=1
    done < <(collect_rs_files)
}

# There must be exactly one `CalibrationId` type in the program, in the
# dependency-free leaf; every other crate re-exports it. A second public or
# private definition would fork the identity space a projection reference and a
# calibration artifact must share, so it is forbidden here (the `\b` stops the
# pattern from matching `CalibrationIdentity`).
check_calibration_id_uniqueness() {
    local canonical="./crates/pilotage-calibration-id/src/lib.rs"
    local matches unexpected file
    matches=""
    while IFS= read -r file; do
        is_excluded_path "$file" && continue
        if grep -Eq 'struct[[:space:]]+CalibrationId\b' "$file"; then
            matches="$matches$file"$'\n'
        fi
    done < <(collect_rs_files)

    if ! printf '%s' "$matches" | grep -qxF "$canonical"; then
        echo "FORBIDDEN: canonical CalibrationId not found at $canonical" >&2
        status=1
    fi
    unexpected="$(printf '%s' "$matches" | grep -vxF "$canonical" || true)"
    while IFS= read -r file; do
        [ -z "$file" ] && continue
        echo "FORBIDDEN: $file defines a second CalibrationId; the only definition belongs in $canonical" >&2
        status=1
    done <<< "$unexpected"
}

check_forbidden_filenames
check_file_length
check_function_length
check_calibration_id_uniqueness

if [ "$status" -ne 0 ]; then
    echo "check-structure: FAILED" >&2
    exit 1
fi

echo "check-structure: OK"
