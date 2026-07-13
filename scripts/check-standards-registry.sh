#!/usr/bin/env bash
# Guards the standards registry (AIR-04) and its agreement with the
# human-readable applicability matrix.
#
# docs/instruments/standards-registry.toml is the machine-checkable source of
# truth for the vision-guidance references whose active revision and
# supersession state must not drift from standards-applicability.md. This check
# fails CI when:
#
#   - the registry is absent or contains no reference entries (fail-closed: a
#     data outage is never reported green);
#   - an entry is missing status provenance (identity, selected revision,
#     authority status, publisher URL, or a well-formed verified_on date), or
#     carries an authority status outside the controlled vocabulary;
#   - a revision is classified as both active and superseded (contradiction);
#   - the matrix's revision or authority-status cells diverge from the registry
#     (generated-table drift).
#
# It verifies internal consistency and matrix/registry agreement only. It does
# NOT fetch any URL and cannot prove a revision is still current: external
# freshness is a periodic expert and source review, never an automatic CI
# claim.
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root_dir"

registry="${PILOTAGE_STANDARDS_REGISTRY:-docs/instruments/standards-registry.toml}"
matrix="${PILOTAGE_STANDARDS_MATRIX:-docs/instruments/standards-applicability.md}"
status=0
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

fail_closed() {
    echo "$1" >&2
    echo "check-standards-registry: FAILED" >&2
    exit 1
}

# Fail-closed: an absent registry must never pass. A guard that goes green when
# its input is missing proves nothing.
[ -f "$registry" ] || fail_closed "MISSING REGISTRY: $registry does not exist (fail-closed)"

# Parse the controlled TOML shape (array of [[reference]] tables, one
# key = "value" per line) into a fixed-order TSV. The registry format is owned
# by this repository, so a deterministic field extractor is sufficient and needs
# no external TOML tooling or network.
parsed="$tmp_dir/parsed.tsv"
awk '
    function flush(   ) {
        if (have) {
            printf "%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n",
                f["std_id"], f["title"], f["selected_revision"],
                f["authority_status"], f["publisher_url"], f["secondary_url"],
                f["supersedes"], f["authority_recognition"], f["verified_on"]
        }
    }
    /^[[:space:]]*#/ { next }
    /^[[:space:]]*\[\[reference\]\][[:space:]]*$/ {
        flush()
        split("", f)
        have = 1
        next
    }
    /^[[:space:]]*[A-Za-z_]+[[:space:]]*=/ {
        key = $0
        sub(/[[:space:]]*=.*$/, "", key)
        gsub(/[[:space:]]/, "", key)
        q1 = index($0, "\"")
        val = ""
        if (q1 > 0) {
            rest = substr($0, q1 + 1)
            last = 0
            for (i = length(rest); i >= 1; i--) {
                if (substr(rest, i, 1) == "\"") { last = i; break }
            }
            if (last > 0) { val = substr(rest, 1, last - 1) }
        }
        f[key] = val
        next
    }
    END { flush() }
' "$registry" > "$parsed"

# Fail-closed: a present-but-empty registry is as unacceptable as an absent one.
[ -s "$parsed" ] || fail_closed "EMPTY REGISTRY: $registry contains no [[reference]] entries (fail-closed)"

# Missing status provenance and controlled-vocabulary violations.
awk -F '\t' '
    {
        std = $1; title = $2; sel = $3; auth = $4; pub = $5
        sec = $6; sup = $7; rec = $8; ver = $9
        if (std == "") {
            print "MISSING PROVENANCE: a reference entry has an empty std_id" > "/dev/stderr"
            bad = 1; next
        }
        if (title == "") { printf "MISSING PROVENANCE: %s has no title\n", std > "/dev/stderr"; bad = 1 }
        if (sel == "")   { printf "MISSING PROVENANCE: %s has no selected_revision\n", std > "/dev/stderr"; bad = 1 }
        if (auth == "")  { printf "MISSING PROVENANCE: %s has no authority_status\n", std > "/dev/stderr"; bad = 1 }
        if (pub == "")   { printf "MISSING PROVENANCE: %s has no publisher_url\n", std > "/dev/stderr"; bad = 1 }
        if (ver == "")   { printf "MISSING PROVENANCE: %s has no verified_on date\n", std > "/dev/stderr"; bad = 1 }
        if (ver != "" && ver !~ /^[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]$/) {
            printf "MISSING PROVENANCE: %s verified_on \"%s\" is not YYYY-MM-DD\n", std, ver > "/dev/stderr"; bad = 1
        }
        if (auth != "" &&
            auth != "authority-accepted" &&
            auth != "latest engineering baseline" &&
            auth != "requires authority agreement" &&
            auth != "not applicable") {
            printf "MISSING PROVENANCE: %s authority_status \"%s\" is not in the controlled vocabulary\n", std, auth > "/dev/stderr"; bad = 1
        }
        if (pub != "" && pub !~ /^https:\/\//) {
            printf "MISSING PROVENANCE: %s publisher_url is not an https URL\n", std > "/dev/stderr"; bad = 1
        }
        if (sec != "" && sec !~ /^https:\/\//) {
            printf "MISSING PROVENANCE: %s secondary_url is not an https URL\n", std > "/dev/stderr"; bad = 1
        }
        # A latest-engineering-baseline revision is ahead of any recognizing
        # authority guidance, so its recognition disposition must be explicit
        # rather than implied.
        if (auth == "latest engineering baseline" && rec != "unresolved") {
            printf "MISSING PROVENANCE: %s is \"latest engineering baseline\" but authority_recognition is not \"unresolved\"\n", std > "/dev/stderr"; bad = 1
        }
    }
    END { exit bad }
' "$parsed" || status=1

# Contradiction: a revision the registry records as superseded must never be the
# selected active revision anywhere.
awk -F '\t' '
    {
        n++; std[n] = $1; sel[n] = $3; sup[n] = $7
        if ($7 != "") supset[$7] = $1
    }
    END {
        for (i = 1; i <= n; i++) {
            if (sup[i] != "" && sup[i] == sel[i]) {
                printf "CONTRADICTION: %s selects \"%s\", which it also records as superseded\n", std[i], sel[i] > "/dev/stderr"; bad = 1
            }
            if (sel[i] in supset) {
                printf "CONTRADICTION: %s selects \"%s\", which the registry records as superseded (by %s); a superseded revision cannot be active\n", std[i], sel[i], supset[sel[i]] > "/dev/stderr"; bad = 1
            }
        }
        exit bad
    }
' "$parsed" || status=1

# Generated-table drift: each governed matrix row must name the registry's
# selected revision and authority status, and must never present a superseded
# revision as the selected one.
if [ ! -f "$matrix" ]; then
    echo "DRIFT: matrix $matrix does not exist" >&2
    status=1
else
    awk -F '\t' '
        function trim(s) { gsub(/^[[:space:]]+|[[:space:]]+$/, "", s); return s }
        function split_row(row, out,   tmp, k, i, m) {
            k = split(row, tmp, "|")
            m = 0
            for (i = 2; i < k; i++) { m++; out[m] = tmp[i] }
            return m
        }
        # True when needle occurs in hay as a standalone token (not immediately
        # flanked by an alphanumeric), so "AC 20-185" does not match inside the
        # active "AC 20-185A".
        function has_token(hay, needle,   start, pos, absolute, before, after) {
            start = 1
            while (1) {
                pos = index(substr(hay, start), needle)
                if (pos == 0) return 0
                absolute = start + pos - 1
                before = (absolute > 1) ? substr(hay, absolute - 1, 1) : ""
                after = substr(hay, absolute + length(needle), 1)
                if (before !~ /[A-Za-z0-9]/ && after !~ /[A-Za-z0-9]/) return 1
                start = absolute + 1
            }
        }
        FNR == NR {
            rstd[$1] = 1; rsel[$1] = $3; rauth[$1] = $4
            rpub[$1] = $5; rsec[$1] = $6
            if ($7 != "") superseded[$7] = 1
            next
        }
        {
            line = $0
            if (line ~ /^\|[-: |]+\|$/ && line ~ /-/) {
                header = prevline
                split("", colidx)
                ncol = split_row(header, hcells)
                for (i = 1; i <= ncol; i++) { colidx[trim(hcells[i])] = i }
            } else if (line ~ /^\|[[:space:]]*STD-[0-9][0-9]*[[:space:]]*\|/) {
                split_row(line, dcells)
                id = trim(dcells[1])
                if (id in rstd) {
                    seen[id] = 1
                    sidx = colidx["Selected revision"]
                    aidx = colidx["Authority status"]
                    if (sidx == "" || sidx == 0) {
                        printf "DRIFT: %s row has no \"Selected revision\" column\n", id > "/dev/stderr"; bad = 1
                    } else {
                        selcell = dcells[sidx]
                        np = split(rsel[id], parts, " / ")
                        for (p = 1; p <= np; p++) {
                            if (!has_token(selcell, parts[p])) {
                                printf "DRIFT: %s \"Selected revision\" cell (%s) does not name registry revision \"%s\"\n", id, trim(selcell), rsel[id] > "/dev/stderr"; bad = 1
                            }
                        }
                        for (d in superseded) {
                            if (has_token(selcell, d)) {
                                printf "DRIFT: %s \"Selected revision\" cell names superseded revision \"%s\"\n", id, d > "/dev/stderr"; bad = 1
                            }
                        }
                    }
                    if (aidx != "" && aidx != 0) {
                        authcell = dcells[aidx]
                        if (index(authcell, rauth[id]) == 0) {
                            printf "DRIFT: %s \"Authority status\" cell (%s) does not match registry status \"%s\"\n", id, trim(authcell), rauth[id] > "/dev/stderr"; bad = 1
                        }
                    }
                    if (rpub[id] != "" && index(line, rpub[id]) == 0) {
                        printf "DRIFT: %s row does not carry its registry publisher_url\n", id > "/dev/stderr"; bad = 1
                    }
                    if (rsec[id] != "" && index(line, rsec[id]) == 0) {
                        printf "DRIFT: %s row does not carry its registry secondary_url\n", id > "/dev/stderr"; bad = 1
                    }
                }
            }
            prevline = line
        }
        END {
            for (id in rstd) {
                if (!(id in seen)) {
                    printf "DRIFT: %s from the registry has no matching row in %s\n", id, FILENAME > "/dev/stderr"; bad = 1
                }
            }
            exit bad
        }
    ' "$parsed" "$matrix" || status=1
fi

if [ "${PILOTAGE_STANDARDS_SELFTEST_CHILD:-0}" != "1" ]; then
    "$root_dir/scripts/test-standards-registry.sh" || status=1
fi

if [ "$status" -ne 0 ]; then
    echo "check-standards-registry: FAILED" >&2
    exit 1
fi

count="$(wc -l < "$parsed" | tr -d ' ')"
echo "check-standards-registry: OK ($count references; consistency and matrix agreement only)"
echo "check-standards-registry: external freshness requires periodic expert/source review and is NOT proven by CI"
