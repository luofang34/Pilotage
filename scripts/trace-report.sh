#!/usr/bin/env bash
# Emits a requirement / test / review / configuration inventory to the CI log.
#
# This is an INVENTORY, not a compliance trace. It counts and lists artifacts
# that exist in the tree and reports review closure status verbatim. It does not
# assert that any requirement is verified, any objective satisfied, or any review
# complete, and it fabricates nothing: missing or pending evidence is reported as
# missing or pending. A real certification trace is future lifecycle work (see
# docs/instruments/evidence-plan.md).
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root_dir"

document_dir="${PILOTAGE_INSTRUMENT_DOCUMENT_DIR:-docs/instruments}"
catalog="$document_dir/requirements.md"
review_record="$document_dir/review-record.md"
pssa="$document_dir/pssa.md"

if [ ! -f "$catalog" ]; then
    echo "trace-report: missing $catalog" >&2
    exit 1
fi

echo "=== Pilotage instrument trace inventory (NOT a compliance trace) ==="
echo "Nothing below asserts verification or approval; pending evidence is"
echo "reported as pending. See docs/instruments/evidence-plan.md."
echo

echo "--- Requirements (docs/instruments/requirements.md) ---"
defined="$(grep -cE '^### AIR-[A-Z0-9]+-[0-9]{3} — ' "$catalog" || true)"
echo "Defined requirement identifiers: $defined"
# Unique identifiers referenced anywhere in the instrument docs (references are
# occurrences outside the catalog's own definition headers and anchors).
referenced="$(
    {
        grep -REo 'AIR-[A-Z0-9]+-[0-9]{3}' "$document_dir" --include='*.md' \
            | sed 's/^.*://'
    } | LC_ALL=C sort -u | wc -l | tr -d ' '
)"
echo "Distinct identifiers appearing across the docs: $referenced"
echo "Requirement families defined:"
grep -oE '^### AIR-[A-Z0-9]+-[0-9]{3}' "$catalog" \
    | sed -E 's/^### (AIR-[A-Z0-9]+)-[0-9]{3}/\1/' \
    | LC_ALL=C sort | uniq -c \
    | sed 's/^/    /'
echo

echo "--- Tests present in the tree (inventory of artifacts, not results) ---"
rust_tests="$(git ls-files '**/tests.rs' '**/tests/*.rs' | wc -l | tr -d ' ')"
browser_suites="$(git ls-files 'clients/web/*.test.mjs' | wc -l | tr -d ' ')"
echo "Rust test modules (tests.rs + tests/*.rs): $rust_tests"
echo "Browser conformance suites (clients/web/*.test.mjs): $browser_suites"
echo "Named test artifacts referenced from the docs:"
grep -rhoE '[A-Za-z0-9_-]+\.test\.mjs|pilotage-instrument-[a-z]+' \
    "$document_dir" --include='*.md' \
    | LC_ALL=C sort -u \
    | sed 's/^/    /' || true
echo

echo "--- Reviews (closure status, reported verbatim) ---"
for record in "$review_record" "$pssa"; do
    [ -f "$record" ] || continue
    pending="$(grep -cE 'PENDING' "$record" || true)"
    echo "$record: PENDING fields = $pending"
done
if [ -f "$review_record" ]; then
    echo "AIR-01 closure decision lines:"
    grep -E 'Tracking issue may close:|All three reviews complete:' "$review_record" \
        | sed 's/^/    /' || true
fi
echo

echo "--- Configuration record (git as the engineering configuration log) ---"
head_sha="$(git rev-parse --short HEAD 2>/dev/null || echo 'unknown')"
adr_count="$(git ls-files 'docs/adr/0*.md' | wc -l | tr -d ' ')"
echo "HEAD commit: $head_sha"
echo "Architecture decision records: $adr_count"
echo "Structural/requirement guards in force:"
for guard in scripts/check-structure.sh scripts/check-instrument-requirements.sh scripts/check-certification-claims.sh; do
    [ -f "$guard" ] && echo "    $guard"
done
echo

echo "trace-report: inventory emitted (informational; not a compliance trace)"
