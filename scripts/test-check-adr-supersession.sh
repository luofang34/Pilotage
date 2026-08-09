#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
checker="${script_dir}/check-adr-supersession.sh"
fixture_root="$(mktemp -d "${TMPDIR:-/tmp}/pilotage-adr-lifecycle.XXXXXX")"
trap 'rm -rf -- "$fixture_root"' EXIT

tests_run=0

fail() {
    echo "test-check-adr-supersession: $*" >&2
    exit 1
}

new_fixture() {
    local name="$1"
    local root="${fixture_root}/${name}"

    mkdir -p "$root"
    printf '# Architecture Decision Records\n\n## Index\n\n| ADR | Decision | Status |\n|---|---|---|\n' >"${root}/README.md"
    printf '%s\n' "$root"
}

write_adr() {
    local root="$1"
    local number="$2"
    local slug="$3"
    local status="$4"
    local predecessor_number="${5:-}"
    local predecessor_file="${6:-}"
    local narrative="${7:-Fixture context.}"
    local file="${root}/${number}-${slug}.md"

    {
        printf '# ADR-%s: Fixture\n\n' "$number"
        printf -- '- Status: %s\n' "$status"
        if [ -n "$predecessor_number" ]; then
            printf -- '- Supersedes on acceptance: [ADR-%s](%s)\n' \
                "$predecessor_number" "$predecessor_file"
        fi
        printf '\n## Context\n\n%s\n' "$narrative"
    } >"$file"
}

add_index_row() {
    local root="$1"
    local number="$2"
    local file="$3"
    local status="$4"

    printf '| [%s](%s) | Fixture %s | %s |\n' "$number" "$file" "$number" "$status" \
        >>"${root}/README.md"
}

expect_success_env() {
    local name="$1"
    local root="$2"
    local output

    tests_run=$((tests_run + 1))
    if ! output="$(ADR_ROOT="$root" bash "$checker" 2>&1)"; then
        fail "${name} failed unexpectedly:\n${output}"
    fi
}

expect_success_arg() {
    local name="$1"
    local root="$2"
    local output

    tests_run=$((tests_run + 1))
    if ! output="$(bash "$checker" "$root" 2>&1)"; then
        fail "${name} failed unexpectedly:\n${output}"
    fi
}

expect_failure() {
    local name="$1"
    local root="$2"
    local expected="$3"
    local output

    tests_run=$((tests_run + 1))
    if output="$(bash "$checker" "$root" 2>&1)"; then
        fail "${name} succeeded unexpectedly:\n${output}"
    fi
    if [[ "$output" != *"$expected"* ]]; then
        fail "${name} did not report '${expected}':\n${output}"
    fi
}

root="$(new_fixture proposed_success)"
write_adr "$root" 0001 first Accepted
write_adr "$root" 0002 second Proposed 0001 0001-first.md
add_index_row "$root" 0001 0001-first.md Accepted
add_index_row "$root" 0002 0002-second.md Proposed
expect_success_env proposed_success "$root"

root="$(new_fixture proposed_predecessor_not_accepted)"
write_adr "$root" 0001 first "Superseded by ADR-0002"
write_adr "$root" 0002 second Proposed 0001 0001-first.md
add_index_row "$root" 0001 0001-first.md "Superseded by ADR-0002"
add_index_row "$root" 0002 0002-second.md Proposed
expect_failure proposed_predecessor_not_accepted "$root" \
    "proposed ADR-0002 requires ADR-0001 to be Accepted"

root="$(new_fixture accepted_success)"
write_adr "$root" 0001 first "Superseded by ADR-0002"
write_adr "$root" 0002 second Accepted 0001 0001-first.md
add_index_row "$root" 0001 0001-first.md "Superseded by ADR-0002"
add_index_row "$root" 0002 0002-second.md Accepted
expect_success_arg accepted_success "$root"

root="$(new_fixture narrative_ignored)"
write_adr "$root" 0001 first Accepted "" "" \
    "This narrative says Superseded by ADR-9999, but it is not status metadata."
add_index_row "$root" 0001 0001-first.md Accepted
expect_success_arg narrative_ignored "$root"

root="$(new_fixture readme_mismatch)"
write_adr "$root" 0001 first "Superseded by ADR-0002"
write_adr "$root" 0002 second Accepted 0001 0001-first.md
add_index_row "$root" 0001 0001-first.md "Superseded by ADR-0002"
add_index_row "$root" 0002 0002-second.md Proposed
expect_failure readme_mismatch "$root" "README status for ADR-0002 is 'Proposed'"

root="$(new_fixture missing_target)"
write_adr "$root" 0001 first "Superseded by ADR-0002"
add_index_row "$root" 0001 0001-first.md "Superseded by ADR-0002"
expect_failure missing_target "$root" "ADR-0002 has no file"

root="$(new_fixture chain)"
write_adr "$root" 0001 first "Superseded by ADR-0002"
write_adr "$root" 0002 second "Superseded by ADR-0003" 0001 0001-first.md
write_adr "$root" 0003 third Accepted 0002 0002-second.md
add_index_row "$root" 0001 0001-first.md "Superseded by ADR-0002"
add_index_row "$root" 0002 0002-second.md "Superseded by ADR-0003"
add_index_row "$root" 0003 0003-third.md Accepted
expect_success_arg chain "$root"

echo "test-check-adr-supersession: OK (${tests_run} cases)"
