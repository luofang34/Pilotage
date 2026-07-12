#!/usr/bin/env bash
# Exercises failure modes that a successful repository-only check cannot reach.
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
checker="$root_dir/scripts/check-instrument-requirements.sh"
source_dir="$root_dir/docs/instruments"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

prepare_case() {
    case_dir="$tmp_dir/$1"
    mkdir -p "$case_dir"
    cp -R "$source_dir/." "$case_dir"
}

run_checker() {
    env \
        "PILOTAGE_INSTRUMENT_DOCUMENT_DIR=$case_dir" \
        PILOTAGE_INSTRUMENT_SELFTEST_CHILD=1 \
        bash "$checker"
}

show_failure() {
    echo "instrument-requirements-selftest: $1" >&2
    sed 's/^/    /' "$output_file" >&2
    exit 1
}

expect_success() {
    output_file="$tmp_dir/$1.output"
    if ! run_checker > "$output_file" 2>&1; then
        show_failure "$1 unexpectedly failed"
    fi
}

expect_failure() {
    output_file="$tmp_dir/$1.output"
    if run_checker > "$output_file" 2>&1; then
        show_failure "$1 unexpectedly passed"
    fi
    if ! grep -Fq "$2" "$output_file"; then
        show_failure "$1 did not report: $2"
    fi
}

prepare_case baseline
expect_success baseline

prepare_case short-id
printf "\n[\`AIR-TST-01\`](requirements.md#air-tst-01)\n" \
    >> "$case_dir/intended-functions.md"
expect_failure short-id 'MALFORMED REQUIREMENT ID:'

prepare_case long-id
printf "\n[\`AIR-TST-0001\`](requirements.md#air-tst-0001)\n" \
    >> "$case_dir/intended-functions.md"
expect_failure long-id 'MALFORMED REQUIREMENT ID:'

prepare_case orphan
printf '\n<a id="air-tst-999"></a>\n### AIR-TST-999 — Self-test orphan\n' \
    >> "$case_dir/requirements.md"
expect_failure orphan 'UNREFERENCED REQUIREMENT: AIR-TST-999'

prepare_case valid-range
printf '\n<a id="air-tst-991"></a>\n### AIR-TST-991 — Range start\n<a id="air-tst-992"></a>\n### AIR-TST-992 — Range member\n<a id="air-tst-993"></a>\n### AIR-TST-993 — Range end\n' \
    >> "$case_dir/requirements.md"
printf "\n[\`AIR-TST-991\`](requirements.md#air-tst-991) through [\`AIR-TST-993\`](requirements.md#air-tst-993)\n" \
    >> "$case_dir/intended-functions.md"
expect_success valid-range

prepare_case false-range
printf '\n<a id="air-tst-981"></a>\n### AIR-TST-981 — Phrase start\n<a id="air-tst-982"></a>\n### AIR-TST-982 — Unreferenced member\n<a id="air-tst-983"></a>\n### AIR-TST-983 — Phrase end\n' \
    >> "$case_dir/requirements.md"
printf "\n[\`AIR-TST-981\`](requirements.md#air-tst-981) is evaluated through a separate process before [\`AIR-TST-983\`](requirements.md#air-tst-983).\n" \
    >> "$case_dir/intended-functions.md"
expect_failure false-range 'UNREFERENCED REQUIREMENT: AIR-TST-982'

echo "instrument-requirements-selftest: OK (6 cases)"
