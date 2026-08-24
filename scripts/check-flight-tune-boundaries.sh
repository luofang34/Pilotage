#!/usr/bin/env bash
# Prevent simulator adapter details from entering shared tuning contracts.
set -euo pipefail

default_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
root="${1:-$default_root}"
root="$(cd "$root" && pwd)"
allowlist_is_override=0
if [ "$#" -ge 2 ]; then
    allowlist="$2"
    allowlist_is_override=1
else
    allowlist="$root/scripts/flight-tune-xplane-import-allowlist.tsv"
fi
work="$(mktemp -d "${TMPDIR:-/tmp}/pilotage-flight-tune-boundary.XXXXXX")"
trap 'rm -rf "$work"' EXIT

status=0

report() {
    echo "FORBIDDEN: $1" >&2
    status=1
}

require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "check-flight-tune-boundaries: required command $1 is not available" >&2
        exit 1
    fi
}

relative_path() {
    printf '%s\n' "${1#"$root"/}"
}

is_production_rust_path() {
    case "$1" in
        */test_support.rs|*/test_support/*|*/tests.rs|*/tests/*) return 1 ;;
        *) return 0 ;;
    esac
}

collect_production_files() {
    local directory file list_id list_path list_status
    list_id=0
    for directory in "$@"; do
        [ -d "$directory" ] || continue
        list_id=$((list_id + 1))
        list_path="$work/source-list-$list_id"
        list_status=0
        find "$directory" -type f -name '*.rs' -print > "$list_path" \
            || list_status=$?
        if [ "$list_status" -ne 0 ]; then
            report "cannot list Rust source files below $(relative_path "$directory")"
            continue
        fi
        while IFS= read -r file; do
            if is_production_rust_path "$file"; then
                printf '%s\n' "$file"
            fi
        done < "$list_path"
    done
}

collect_file_imports() {
    local file="$1" relative="$2" matches="$3" match_status
    if ! awk -v path="$relative" '
        BEGIN { collecting = 0; statement = "" }
        /^[[:space:]]*(pub(\([^)]*\))?[[:space:]]+)?use[[:space:]]+(::)?flight_tune_xplane(::|[[:space:]]|;)/ {
            collecting = 1
        }
        collecting {
            statement = statement $0
            if ($0 ~ /;/) {
                gsub(/[[:space:]]/, "", statement)
                print path "\t" statement
                collecting = 0
                statement = ""
            }
        }
        END { if (collecting) exit 2 }
    ' "$file" >> "$work/aviate-imports"; then
        report "$relative has an incomplete flight_tune_xplane import"
    fi

    match_status=0
    grep -n 'flight_tune_xplane' "$file" > "$matches" || match_status=$?
    if [ "$match_status" -gt 1 ]; then
        report "$relative cannot be scanned for flight_tune_xplane references"
        return
    fi
    while IFS=: read -r line_number source; do
        [ -n "$line_number" ] || continue
        if ! grep -Eq '^[[:space:]]*(pub(\([^)]*\))?[[:space:]]+)?use[[:space:]]+(::)?flight_tune_xplane(::|[[:space:]]|;)' <<<"$source"; then
            report "$relative:$line_number uses flight_tune_xplane outside a reviewed import"
        fi
    done < "$matches"
}

collect_aviate_imports() {
    local file relative scan_id
    collect_production_files "$root/tools/flight-tune-aviate/src" > "$work/aviate-files-unsorted"
    LC_ALL=C sort -u "$work/aviate-files-unsorted" > "$work/aviate-files"
    : > "$work/aviate-imports"
    scan_id=0
    while IFS= read -r file; do
        [ -n "$file" ] || continue
        scan_id=$((scan_id + 1))
        relative="$(relative_path "$file")"
        collect_file_imports "$file" "$relative" "$work/import-matches-$scan_id"
    done < "$work/aviate-files"
    LC_ALL=C sort -u "$work/aviate-imports" -o "$work/aviate-imports"
}

check_aviate_imports() {
    collect_aviate_imports
    if [ ! -f "$allowlist" ]; then
        report "the flight_tune_xplane import allowlist is missing"
        : > "$work/allowed-imports"
    else
        awk 'NF > 0 && $1 !~ /^#/ { print }' "$allowlist" \
            | LC_ALL=C sort -u > "$work/allowed-imports"
        if [ "$allowlist_is_override" -eq 0 ] && [ -s "$work/allowed-imports" ]; then
            report "the production flight_tune_xplane import allowlist must stay empty"
        fi
    fi
    if ! comm -23 "$work/aviate-imports" "$work/allowed-imports" \
        > "$work/new-imports"; then
        report "cannot compare flight_tune_xplane imports with the allowlist"
        return
    fi
    while IFS= read -r import; do
        [ -n "$import" ] && report "new flight_tune_xplane import: $import"
    done < "$work/new-imports"
}

check_shared_contracts() {
    if ! python3 "$default_root/scripts/check-flight-tune-contracts.py" "$root"; then
        status=1
    fi
}

require_command find
require_command grep
require_command cargo
require_command python3
require_command comm
check_aviate_imports
check_shared_contracts

if [ "$status" -ne 0 ]; then
    echo "check-flight-tune-boundaries: FAILED" >&2
    exit 1
fi

echo "check-flight-tune-boundaries: OK"
