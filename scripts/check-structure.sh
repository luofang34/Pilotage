#!/usr/bin/env bash
# Enforces the structural limits from ADR-0015 that are not expressible as
# rustc/clippy lints:
#   - no mod.rs files
#   - no utils.rs / helpers.rs / common.rs files
#   - no first-party .rs file over 500 lines (excluding target/ and any
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
    git ls-files --cached --others --exclude-standard -- '*.rs' \
        | sed 's#^#./#'
}

collect_swift_files() {
    git ls-files --cached --others --exclude-standard -- '*.swift' \
        | sed 's#^#./#'
}

# Swift sources obey the same file-length ceiling as Rust. Function
# bodies are not measured here: Swift's syntax needs a real parser, and
# a wrong count is worse than none. Files named here carried the debt
# before the ceiling watched Swift at all; each is pinned at its
# recorded count and may only shrink.
swift_length_ceiling() {
    case "$1" in
        ./clients/apple/App/HostLinkModel.swift) echo 676 ;;
        *) echo 500 ;;
    esac
}

check_swift_file_lengths() {
    local file lines limit
    while IFS= read -r file; do
        is_excluded_path "$file" && continue
        limit="$(swift_length_ceiling "$file")"
        lines="$(wc -l < "$file" | tr -d ' ')"
        if [ "$lines" -gt "$limit" ]; then
            echo "FORBIDDEN: $file has $lines lines (limit $limit)" >&2
            status=1
        fi
    done < <(collect_swift_files)
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

# The palette names RED, AMBER, YELLOW, and BAND_YELLOW alias the
# never-skinnable safety set (ADR-0029). Outside the symbology crate —
# a pinned upstream dependency here — safety-semantic paints must
# reference `safety::` directly so a future palette-to-theme sweep
# cannot silently make failure, caution, or reference colors skinnable.
# Text-level ratchet, not a proof: it catches direct `palette::RED`
# uses and `use ...::palette::RED` imports, but a module alias
# (`use ... as p; p::RED`) slips through, like the AWK heuristics above.
check_safety_palette_aliases() {
    local file
    while IFS= read -r file; do
        is_excluded_path "$file" && continue
        if grep -Eq 'palette::(RED|AMBER|YELLOW|BAND_YELLOW)\b' "$file"; then
            echo "FORBIDDEN: $file references a safety palette alias; use the safety:: constants outside indicate-instrument-symbology" >&2
            status=1
        fi
    done < <(collect_rs_files)
}

# The Indicate pin is one fact recorded in two files: every Indicate git
# dependency in Cargo.toml and the rev the evidence-gate workflow
# installs the gate binary from must be identical, or the gate silently
# builds from a different tree than the workspace links.
check_indicate_pin_coherence() {
    local manifest_revs workflow_rev count
    manifest_revs="$(grep -oE 'Indicate\.git", rev = "[0-9a-f]{40}"' Cargo.toml \
        | grep -oE '[0-9a-f]{40}' | LC_ALL=C sort -u || true)"
    count="$(printf '%s\n' "$manifest_revs" | grep -c . || true)"
    if [ "$count" -ne 1 ]; then
        echo "FORBIDDEN: Cargo.toml pins Indicate at $count distinct revs; the family advances as one" >&2
        status=1
        return
    fi
    workflow_rev="$(grep -oE -- '--rev [0-9a-f]{40}' .github/workflows/evidence-gate.yml \
        | grep -oE '[0-9a-f]{40}' | LC_ALL=C sort -u || true)"
    if [ "$workflow_rev" != "$manifest_revs" ]; then
        echo "FORBIDDEN: evidence-gate.yml installs the gate at rev ${workflow_rev:-<none>} but Cargo.toml pins $manifest_revs; advance them together" >&2
        status=1
    fi
}

case "${1:-}" in
    "")
        check_forbidden_filenames
        check_file_length
        check_swift_file_lengths
        check_function_length
        check_calibration_id_uniqueness
        check_safety_palette_aliases
        check_indicate_pin_coherence
        ;;
    --forbidden-filenames-only)
        check_forbidden_filenames
        ;;
    *)
        echo "usage: $0 [--forbidden-filenames-only]" >&2
        exit 2
        ;;
esac

if [ "$status" -ne 0 ]; then
    echo "check-structure: FAILED" >&2
    exit 1
fi

echo "check-structure: OK"
