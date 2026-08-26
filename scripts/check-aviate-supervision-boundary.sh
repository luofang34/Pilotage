#!/usr/bin/env bash
# Keep destructive process control inside the Aviate supervisor.
set -euo pipefail

default_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
root="${1:-$default_root}"
root="$(cd "$root" && pwd)"
source_root="$root/tools/flight-tune-aviate/src"
interface="tools/flight-tune-aviate/src/supervisor/process_control.rs"
work="$(mktemp -d "${TMPDIR:-/tmp}/pilotage-aviate-supervision-boundary.XXXXXX")"
trap 'rm -rf "$work"' EXIT

status=0

report() {
    echo "FORBIDDEN: $1" >&2
    status=1
}

relative_path() {
    printf '%s\n' "${1#"$root"/}"
}

is_production_rust_path() {
    case "$1" in
        */tests.rs|*/tests/*) return 1 ;;
        *) return 0 ;;
    esac
}

collect_production_files() {
    local file link
    find "$source_root" -type l -print > "$work/links"
    while IFS= read -r link; do
        if is_production_rust_path "$link"; then
            report "production Rust source path $(relative_path "$link") is a symlink"
        fi
    done < "$work/links"
    find "$source_root" -type f -name '*.rs' -print > "$work/files"
    while IFS= read -r file; do
        if is_production_rust_path "$file"; then
            printf '%s\n' "$file"
        fi
    done < "$work/files"
}

check_raw_process_control() {
    local file relative matches match_status
    while IFS= read -r file; do
        relative="$(relative_path "$file")"
        matches="$work/raw-$(printf '%s' "$relative" | tr '/.' '__')"
        match_status=0
        grep -nE \
            'rustix::process::kill_current_process_group|rustix::process::kill_process_group|rustix::process::kill_process[[:space:]]*\(|(std::process::)?Child::kill[[:space:]]*\(|\.kill[[:space:]]*\(' \
            "$file" > "$matches" || match_status=$?
        if [ "$match_status" -gt 1 ]; then
            report "$relative cannot be scanned for process-control primitives"
            continue
        fi
        if [ "$relative" != "$interface" ]; then
            while IFS=: read -r line_number _source; do
                [ -n "$line_number" ] \
                    && report "$relative:$line_number uses a raw process-control primitive"
            done < "$matches"
        fi
    done < "$work/production-files"
}

allowed_entry_point() {
    local relative="$1" reference="$2"
    case "$relative:$reference" in
        tools/flight-tune-aviate/src/supervisor/gate.rs:process_control::stop_child) return 0 ;;
        tools/flight-tune-aviate/src/supervisor/gate.rs:process_control::signal_current_process_group) return 0 ;;
        tools/flight-tune-aviate/src/supervisor/owner/cleanup.rs:process_control::signal_process_group) return 0 ;;
        tools/flight-tune-aviate/src/supervisor/owner/launch.rs:process_control::stop_child) return 0 ;;
        *) return 1 ;;
    esac
}

check_process_control_entry_points() {
    local file relative matches match_status line_number reference
    while IFS= read -r file; do
        relative="$(relative_path "$file")"
        matches="$work/entry-$(printf '%s' "$relative" | tr '/.' '__')"
        match_status=0
        grep -nEo 'process_control::[a-z_][a-z0-9_]*' \
            "$file" > "$matches" || match_status=$?
        if [ "$match_status" -gt 1 ]; then
            report "$relative cannot be scanned for process-control entry points"
            continue
        fi
        while IFS=: read -r line_number reference; do
            [ -n "$line_number" ] || continue
            if ! allowed_entry_point "$relative" "$reference"; then
                report "$relative:$line_number uses an unowned process-control entry point"
            fi
        done < "$matches"
    done < "$work/production-files"
}

if [ ! -f "$root/$interface" ]; then
    report "$interface is missing"
fi
collect_production_files | LC_ALL=C sort -u > "$work/production-files"
check_raw_process_control
check_process_control_entry_points

if [ "$status" -ne 0 ]; then
    echo "check-aviate-supervision-boundary: FAILED" >&2
    exit 1
fi

echo "check-aviate-supervision-boundary: OK"
