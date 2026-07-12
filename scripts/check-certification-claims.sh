#!/usr/bin/env bash
# Fails CI when a tracked documentation or UI artifact asserts a
# certification, compliance, approval, or airworthiness claim about this
# project. Pilotage is SIM / NOT FOR FLIGHT: no artifact may state or imply that
# any Pilotage output is certified, DO-178C compliant, FAA/EASA approved, or
# airworthy.
#
# The standards and safety documents legitimately discuss these words in a
# classification or negation context (e.g. "not certified", or enumerating the
# banned vocabulary itself). They are exempted by an explicit allowlist and, in
# exchange, are required to carry the SIM / NOT FOR FLIGHT banner so the exempt
# set stays honest. Every other scanned artifact fails on the assertive-claim
# patterns and the offending file:line is printed.
#
# This is a guard against misleading claims, not a proof of their absence: the
# patterns target assertive phrasing and can be evaded by unusual wording. Treat
# an addition to the allowlist as a reviewed decision.
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root_dir"

banner="SIM / NOT FOR FLIGHT"

# Documents permitted to contain the claim vocabulary in a classification or
# negation context. Each must carry the banner (checked below).
allowlist=(
    "docs/instruments/standards-applicability.md"
    "docs/instruments/evidence-plan.md"
    "docs/instruments/fha.md"
    "docs/instruments/pssa.md"
)

# Assertive claim patterns (extended regex, matched case-insensitively). Neutral
# uses such as "certification basis", "certification authority", "the certified
# world", "does not claim certification", "continued-airworthiness", and "not a
# compliance model" are intentionally not matched.
claim_pattern='certifiable|airworthy|faa[ -]approved|faa[ -]certified|easa[ -]approved|flight[ -]certified|do-178[abc]?[ -]compliant|compliant with do-178|do-178[abc]?[ -]certified|meets( all)? do-178|fully compliant|(^|[^[:alnum:]])(is|are|now|fully|hereby|been|being|shall be) certified'

is_allowlisted() {
    local candidate="$1" allowed
    for allowed in "${allowlist[@]}"; do
        [ "$candidate" = "$allowed" ] && return 0
    done
    return 1
}

# Tracked documentation and UI artifacts. Rust/source crates are out of scope:
# this guard is about the words the project publishes to humans.
collect_scanned_files() {
    git ls-files \
        'docs/**/*.md' \
        'README.md' \
        'clients/web/*.html' \
        'clients/web/*.js' \
        'clients/web/*.mjs' \
        '.github/**/*.md' \
        '.github/ISSUE_TEMPLATE/*' \
        '.github/PULL_REQUEST_TEMPLATE*' \
        'CHANGELOG*' 'RELEASE*' 'HISTORY*'
}

status=0

while IFS= read -r file; do
    [ -z "$file" ] && continue
    [ -f "$file" ] || continue
    if is_allowlisted "$file"; then
        if ! grep -qF "$banner" "$file"; then
            echo "MISSING BANNER: $file (allowlisted standards doc must carry '$banner')" >&2
            status=1
        fi
        continue
    fi
    if hits="$(grep -HnEi "$claim_pattern" "$file")"; then
        while IFS= read -r hit; do
            echo "CERTIFICATION CLAIM: $hit" >&2
        done <<< "$hits"
        status=1
    fi
done < <(collect_scanned_files | LC_ALL=C sort -u)

if [ "$status" -ne 0 ]; then
    echo "check-certification-claims: FAILED" >&2
    exit 1
fi

echo "check-certification-claims: OK"
