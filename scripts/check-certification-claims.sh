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

# An open question must never quietly satisfy a coverage row: any matrix row
# left unverified/to-verify/to-confirm/TBD must be reconciled by a matching
# STD-keyed entry in the matrix's "Open verification actions" section.
check_open_verification_actions() {
    local matrix="docs/instruments/standards-applicability.md"
    [ -f "$matrix" ] || return 0
    awk '
        # First pass: collect STD ids listed under "Open verification actions".
        FNR == NR {
            if ($0 ~ /^#+[ \t]+Open verification actions[ \t]*$/) { in_actions = 1; next }
            if (in_actions && $0 ~ /^#+[ \t]/) { in_actions = 0 }
            if (in_actions) {
                s = $0
                while (match(s, /STD-[0-9]+/)) {
                    actions[substr(s, RSTART, RLENGTH)] = 1
                    s = substr(s, RSTART + RLENGTH)
                }
            }
            next
        }
        # Second pass: every matrix row carrying an unresolved marker must be keyed.
        /^\|/ && /STD-[0-9]+/ {
            low = tolower($0)
            if (low ~ /to-verify|to verify|unverified|to be verified|to be determined|to[ -]confirm|(^|[^a-z])tbd([^a-z]|$)/) {
                match($0, /STD-[0-9]+/)
                id = substr($0, RSTART, RLENGTH)
                if (!(id in actions)) {
                    printf "UNRESOLVED VERIFICATION: %s:%d %s is marked unverified/to-verify/to-confirm/TBD without a matching entry in the \"Open verification actions\" section\n", FILENAME, FNR, id > "/dev/stderr"
                    bad = 1
                }
            }
        }
        END { exit bad }
    ' "$matrix" "$matrix" || status=1
}

check_open_verification_actions

if [ "$status" -ne 0 ]; then
    echo "check-certification-claims: FAILED" >&2
    exit 1
fi

echo "check-certification-claims: OK"
