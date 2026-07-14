#!/usr/bin/env bash
# Fails CI when a tracked documentation or UI artifact asserts a
# certification, compliance, approval, or airworthiness claim about this
# project. Pilotage is SIM / NOT FOR FLIGHT: no artifact may state or imply that
# any Pilotage output is certified, DO-178C compliant, FAA/EASA approved, or
# airworthy.
#
# The banned vocabulary is legitimately used to DENY such status — the required
# safety wording "Nothing here is certified, approved, or airworthy" is the
# opposite of a claim. An assertive claim is distinguished from a denial per
# sentence: a claim keyword is a violation unless a negation (no/not/nothing/
# never/neither/none/cannot/without) precedes it within the same sentence AND
# the file carries the SIM / NOT FOR FLIGHT banner. Splitting on sentence
# boundaries means a banner sentence cannot license a separate assertive
# sentence sharing the line, and an affirmative "Pilotage is airworthy" still
# fails. The `airworthy`/`certified` vocabulary is never removed and no file is
# broadly exempted; the check stays fail-closed.
#
# Standards/safety documents that ENUMERATE the vocabulary in classification
# context are exempted by an explicit allowlist and, in exchange, must carry the
# banner so the exempt set stays honest.
#
# This is a guard against misleading claims, not a proof of their absence: the
# patterns target assertive phrasing and can be evaded by unusual wording. Treat
# an addition to the allowlist as a reviewed decision.
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root_dir"

banner="SIM / NOT FOR FLIGHT"

# Documents permitted to enumerate the claim vocabulary in classification
# context. Each must carry the banner (checked below).
allowlist=(
    "docs/instruments/standards-applicability.md"
    "docs/instruments/evidence-plan.md"
    "docs/instruments/fha.md"
    "docs/instruments/pssa.md"
)

# Assertive claim patterns (extended regex, matched case-insensitively over
# lowercased text, so `[a-z0-9]` also covers upper case and stays portable to
# awk's dynamic regex). Neutral uses such as "certification basis",
# "certification authority", "does not claim certification", and
# "continued-airworthiness" are intentionally not matched.
claim_pattern='certifiable|airworthy|faa[ -]approved|faa[ -]certified|easa[ -]approved|flight[ -]certified|do-178[abc]?[ -]compliant|compliant with do-178|do-178[abc]?[ -]certified|meets( all)? do-178|fully compliant|(^|[^a-z0-9])(is|are|now|fully|hereby|been|being|shall be) certified'

is_allowlisted() {
    local candidate="$1" allowed
    for allowed in "${allowlist[@]}"; do
        [ "$candidate" = "$allowed" ] && return 0
    done
    return 1
}

# True (exit 0) when every claim keyword on the line is DENIED: a negation
# precedes it within its sentence, so the line disclaims rather than asserts.
# Exit 1 as soon as one claim keyword stands un-negated in its sentence.
line_is_negated_disclaimer() {
    printf '%s\n' "$1" | awk -v claim="$claim_pattern" '
    {
        lc = tolower($0)
        n = split(lc, sent, /[.!?]/)
        for (i = 1; i <= n; i++) {
            seg = sent[i]
            off = 0
            while (match(substr(seg, off + 1), claim)) {
                abs = off + RSTART
                pre = substr(seg, 1, abs - 1)
                if (pre !~ /(^|[^a-z])(no|not|nothing|never|neither|none|cannot|without)([^a-z]|$)/) {
                    exit 1
                }
                off = abs + RLENGTH - 1
            }
        }
        exit 0
    }'
}

# Regression fixtures, executed on every run so CI exercises them (AIR-05).
selftest() {
    local f=0 c
    for c in \
        "SIM / NOT FOR FLIGHT. Nothing here is certified, approved, or airworthy." \
        "It is not certified, not approved, and is not airworthy." \
        "Pilotage is not airworthy and makes no compliance credit."
    do
        line_is_negated_disclaimer "$c" || {
            echo "SELFTEST FAILED: negated disclaimer wrongly flagged: $c" >&2
            f=1
        }
    done
    for c in \
        "Pilotage is airworthy." \
        "Pilotage is certified." \
        "SIM / NOT FOR FLIGHT. Pilotage is airworthy." \
        "The system is now fully compliant." \
        "This build is FAA approved."
    do
        if line_is_negated_disclaimer "$c"; then
            echo "SELFTEST FAILED: assertive claim wrongly exempted: $c" >&2
            f=1
        fi
    done
    return $f
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

if ! selftest; then
    echo "check-certification-claims: self-test regression — the negation/claim discrimination is broken" >&2
    status=1
fi

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
    file_has_banner=0
    grep -qF "$banner" "$file" && file_has_banner=1
    if hits="$(grep -HnEi "$claim_pattern" "$file")"; then
        while IFS= read -r hit; do
            content="${hit#*:}"
            content="${content#*:}"
            if [ "$file_has_banner" -eq 1 ] && line_is_negated_disclaimer "$content"; then
                continue
            fi
            echo "CERTIFICATION CLAIM: $hit" >&2
            status=1
        done <<< "$hits"
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
