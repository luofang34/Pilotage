#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
mkdir -p "$fixture/scripts" "$fixture/src"
cp "$root_dir/scripts/check-monotonic-counter-arithmetic.sh" "$fixture/scripts/"
git -C "$fixture" init -q

printf '%s\n' \
    'pub fn advance(mut index: usize) -> usize {' \
    '    index += 1;' \
    '    index' \
    '}' \
    > "$fixture/src/lib.rs"
printf '%s\t%s\n' \
    'src/lib.rs' \
    'index += 1;' \
    > "$fixture/scripts/monotonic-counter-compound-addition-allowlist.tsv"

bash "$fixture/scripts/check-monotonic-counter-arithmetic.sh" "$fixture" >/dev/null

printf '%s\n' \
    '' \
    'pub fn advance_again(mut index: usize) -> usize {' \
    '    index += 1;' \
    '    index' \
    '}' \
    >> "$fixture/src/lib.rs"
output="$fixture/duplicate.txt"
if bash "$fixture/scripts/check-monotonic-counter-arithmetic.sh" "$fixture" \
    >"$output" 2>&1; then
    echo "the monotonic counter guard accepted a duplicate allowlisted addition" >&2
    exit 1
fi

printf '%s\n' \
    'pub fn advance(mut index: usize) -> usize {' \
    '    index += 1;' \
    '    index' \
    '}' \
    > "$fixture/src/lib.rs"
printf '%s\n' \
    '' \
    'pub fn count(mut events: u64) -> u64 {' \
    '    events += 1;' \
    '    events' \
    '}' \
    >> "$fixture/src/lib.rs"
output="$fixture/failure.txt"
if bash "$fixture/scripts/check-monotonic-counter-arithmetic.sh" "$fixture" \
    >"$output" 2>&1; then
    echo "the monotonic counter guard accepted compound counter arithmetic" >&2
    exit 1
fi
if ! grep -Fq 'events += 1;' "$output"; then
    echo "the monotonic counter guard did not identify the rejected counter" >&2
    exit 1
fi

printf '%s\n' \
    'pub fn advance(mut index: usize) -> usize {' \
    '    index += 1;' \
    '    index' \
    '}' \
    '' \
    'pub fn count(mut events: u64) -> u64 {' \
    '    events = events.wrapping_add(1);' \
    '    events' \
    '}' \
    > "$fixture/src/lib.rs"
bash "$fixture/scripts/check-monotonic-counter-arithmetic.sh" "$fixture" >/dev/null

echo "monotonic counter arithmetic guard self-test: OK"
