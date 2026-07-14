#!/usr/bin/env bash
# Exercises the standards-registry guard against positive and deliberately
# stale/contradictory fixtures, proving the check fails closed on the failure
# modes it claims to detect rather than only passing on good input.
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
checker="$root_dir/scripts/check-standards-registry.sh"
fixtures="$root_dir/scripts/fixtures/standards"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

run_checker() {
    env \
        "PILOTAGE_STANDARDS_REGISTRY=$1" \
        "PILOTAGE_STANDARDS_MATRIX=$2" \
        PILOTAGE_STANDARDS_SELFTEST_CHILD=1 \
        bash "$checker"
}

show_failure() {
    echo "standards-registry-selftest: $1" >&2
    sed 's/^/    /' "$output_file" >&2
    exit 1
}

expect_success() {
    output_file="$tmp_dir/$1.output"
    if ! run_checker "$2" "$3" > "$output_file" 2>&1; then
        show_failure "$1 unexpectedly failed"
    fi
}

expect_failure() {
    output_file="$tmp_dir/$1.output"
    if run_checker "$2" "$3" > "$output_file" 2>&1; then
        show_failure "$1 unexpectedly passed"
    fi
    if ! grep -Fq "$4" "$output_file"; then
        show_failure "$1 did not report: $4"
    fi
}

# The shipped registry must agree with the shipped matrix.
expect_success live \
    "$root_dir/docs/instruments/standards-registry.toml" \
    "$root_dir/docs/instruments/standards-applicability.md"

# A hermetic corrected registry/matrix pair passes.
expect_success good \
    "$fixtures/registry-good.toml" \
    "$fixtures/matrix-good.md"

# A registry that lists the superseded AC 20-167A as active is contradictory.
expect_failure stale-active \
    "$fixtures/registry-stale-active.toml" \
    "$fixtures/matrix-good.md" \
    "CONTRADICTION"

# The same fixture must name the offending superseded revision.
if ! grep -Fq "AC 20-167A" "$tmp_dir/stale-active.output"; then
    output_file="$tmp_dir/stale-active.output"
    show_failure "stale-active did not name AC 20-167A"
fi

# A registry entry without a verified_on date has no status provenance.
expect_failure missing-provenance \
    "$fixtures/registry-missing-provenance.toml" \
    "$fixtures/matrix-good.md" \
    "MISSING PROVENANCE"

# A matrix whose selected-revision cell still names the superseded revision
# drifts from the corrected registry.
expect_failure drift \
    "$fixtures/registry-good.toml" \
    "$fixtures/matrix-drift.md" \
    "DRIFT"

# A matrix that dropped the Authority status column cannot be shown to agree
# with the registry on status, so the guard must fail rather than pass.
expect_failure no-status-column \
    "$fixtures/registry-good.toml" \
    "$fixtures/matrix-no-status.md" \
    "no \"Authority status\" column"

# A matrix whose authority-status cell differs from the registry must fail the
# exact-match comparison.
expect_failure status-mismatch \
    "$fixtures/registry-good.toml" \
    "$fixtures/matrix-status-mismatch.md" \
    "does not exactly match registry status"

# An empty registry must fail closed, never report green on absent data.
expect_failure empty \
    "$fixtures/registry-empty.toml" \
    "$fixtures/matrix-good.md" \
    "EMPTY REGISTRY"

# An absent registry must also fail closed.
expect_failure absent \
    "$tmp_dir/does-not-exist.toml" \
    "$fixtures/matrix-good.md" \
    "MISSING REGISTRY"

echo "standards-registry-selftest: OK (9 cases)"
