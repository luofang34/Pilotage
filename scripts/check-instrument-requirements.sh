#!/usr/bin/env bash
# Verifies the stable instrument requirement registry and its Markdown links.
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root_dir"

document_dir="${PILOTAGE_INSTRUMENT_DOCUMENT_DIR:-docs/instruments}"
catalog="$document_dir/requirements.md"
status=0
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT
definitions="$tmp_dir/definitions"
all_ids="$tmp_dir/all-ids"

if [ ! -f "$catalog" ]; then
    echo "instrument-requirements: missing $catalog" >&2
    exit 1
fi

awk '
    /^### AIR-[A-Z0-9]+-[0-9][0-9][0-9] — / {
        id = $2
        expected = "<a id=\"" tolower(id) "\"></a>"
        if (previous != expected) {
            printf "BROKEN ANCHOR: %s:%d %s requires preceding %s\n", FILENAME, NR, id, expected > "/dev/stderr"
            bad = 1
        }
        printf "%s\t%d\n", id, NR
    }
    { previous = $0 }
    END { exit bad }
' "$catalog" > "$definitions" || status=1

if [ ! -s "$definitions" ]; then
    echo "instrument-requirements: no requirement definitions found" >&2
    status=1
fi

duplicates="$(cut -f1 "$definitions" | LC_ALL=C sort | uniq -d)"
if [ -n "$duplicates" ]; then
    while IFS= read -r id; do
        echo "DUPLICATE REQUIREMENT: $id" >&2
    done <<< "$duplicates"
    status=1
fi

# A requirement token with the wrong digit count would otherwise be
# invisible: too short matches nothing, one digit too many mis-parses as
# a different id plus a stray character. Catch both before extraction.
malformed="$tmp_dir/malformed"
: > "$malformed"
while IFS= read -r file; do
    grep -nEo 'AIR-[A-Z0-9]+-[0-9]+' "$file" \
        | grep -Ev '^[0-9]+:AIR-[A-Z0-9]+-[0-9]{3}$' \
        | sed "s|^|$file:|" >> "$malformed" || true
done < <(find "$document_dir" -type f -name '*.md' -print | LC_ALL=C sort)
grep -nE '^### AIR-' "$catalog" \
    | grep -Ev '^[0-9]+:### AIR-[A-Z0-9]+-[0-9]{3} — ' \
    | sed "s|^|$catalog:|" >> "$malformed" || true
if [ -s "$malformed" ]; then
    while IFS= read -r hit; do
        echo "MALFORMED REQUIREMENT ID: $hit" >&2
    done < "$malformed"
    status=1
fi

while IFS= read -r file; do
    grep -Eo 'AIR-[A-Z0-9]+-[0-9]{3}' "$file" || true
done < <(find "$document_dir" -type f -name '*.md' -print | LC_ALL=C sort) \
    | LC_ALL=C sort -u > "$all_ids"

while IFS= read -r id; do
    if ! cut -f1 "$definitions" | grep -Fqx "$id"; then
        echo "UNDEFINED REQUIREMENT: $id" >&2
        status=1
    fi
done < "$all_ids"

while IFS= read -r file; do
    [ "$file" = "$catalog" ] && continue
    awk '
        {
            rest = $0
            while (match(rest, /AIR-[A-Z0-9]+-[0-9][0-9][0-9]/)) {
                id = substr(rest, RSTART, RLENGTH)
                expected = "[`" id "`](requirements.md#" tolower(id) ")"
                prefix = substr(rest, RSTART - 2, 2)
                suffix = substr(rest, RSTART + RLENGTH)
                expected_suffix = "`](requirements.md#" tolower(id) ")"
                if (RSTART < 3 || prefix != "[`" || index(suffix, expected_suffix) != 1) {
                    printf "BROKEN REQUIREMENT LINK: %s:%d %s must use %s\n", FILENAME, NR, id, expected > "/dev/stderr"
                    bad = 1
                }
                rest = substr(rest, RSTART + RLENGTH)
            }
        }
        END { exit bad }
    ' "$file" || status=1
done < <(find "$document_dir" -type f -name '*.md' -print | LC_ALL=C sort)

# Every definition must be reachable from at least one reference, so a
# requirement cannot silently drop out of the documentation set. A
# reference is an occurrence outside the definition header/anchor, or
# membership in a documented "A through B" span (ranges may wrap lines).
referenced="$tmp_dir/referenced"
{
    while IFS= read -r file; do
        if [ "$file" = "$catalog" ]; then
            # Definition headers and anchors are not references; catalog
            # prose can cross-reference, but "A through B" spans are only
            # trusted from the narrative documents (a requirement body
            # using the word "through" must not fabricate a span).
            grep -Ev '^### AIR-|^<a id=' "$file" | grep -Eo 'AIR-[A-Z0-9]+-[0-9]{3}' || true
            continue
        fi
        grep -Eo 'AIR-[A-Z0-9]+-[0-9]{3}' "$file" || true
        tr '\n' ' ' < "$file" | awk '
            {
                rest = $0
                n = 0
                while (match(rest, /AIR-[A-Z0-9]+-[0-9][0-9][0-9]/)) {
                    n += 1
                    ids[n] = substr(rest, RSTART, RLENGTH)
                    rest = substr(rest, RSTART + RLENGTH)
                    gaps[n] = rest
                }
                for (i = 1; i < n; i++) {
                    split(ids[i], a, "-")
                    split(ids[i + 1], b, "-")
                    between = substr(gaps[i], 1, length(gaps[i]) - length(gaps[i + 1]) - length(ids[i + 1]))
                    normalized = between
                    gsub(/[[:space:]]+/, " ", normalized)
                    expected = "`](requirements.md#" tolower(ids[i]) ") through [`"
                    if (a[2] == b[2] && a[3] + 0 < b[3] + 0 && normalized == expected) {
                        for (k = a[3] + 0; k <= b[3] + 0; k++) {
                            printf "AIR-%s-%03d\n", a[2], k
                        }
                    }
                }
            }
        '
    done < <(find "$document_dir" -type f -name '*.md' -print | LC_ALL=C sort)
} | LC_ALL=C sort -u > "$referenced"

while IFS= read -r id; do
    if ! grep -Fqx "$id" "$referenced"; then
        echo "UNREFERENCED REQUIREMENT: $id" >&2
        status=1
    fi
done < <(cut -f1 "$definitions")

if [ "${PILOTAGE_INSTRUMENT_SELFTEST_CHILD:-0}" != "1" ]; then
    "$root_dir/scripts/test-instrument-requirements.sh" || status=1
fi

if [ "$status" -ne 0 ]; then
    echo "instrument-requirements: FAILED" >&2
    exit 1
fi

count="$(wc -l < "$definitions" | tr -d ' ')"
echo "instrument-requirements: OK ($count unique requirements)"
