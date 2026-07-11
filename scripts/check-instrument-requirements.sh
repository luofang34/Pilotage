#!/usr/bin/env bash
# Verifies the stable instrument requirement registry and its Markdown links.
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root_dir"

catalog="docs/instruments/requirements.md"
document_dir="docs/instruments"
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

if [ "$status" -ne 0 ]; then
    echo "instrument-requirements: FAILED" >&2
    exit 1
fi

count="$(wc -l < "$definitions" | tr -d ' ')"
echo "instrument-requirements: OK ($count unique requirements)"
