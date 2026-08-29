#!/usr/bin/env bash
# A generated scenario corpus is only evidence while it is still the generator's
# output. This runs the generator from its canonical inputs and refuses any
# artifact the checked-in corpus states differently, so nobody can edit one
# condition file, change one version number by hand, or leave an artifact behind
# that no cell schedules.
#
# The Rust half of the same guarantee decodes every artifact with the trial
# contract and requires the current schema. A shell script cannot decode a
# condition; a byte comparison cannot notice a stale schema. Both halves are
# needed and each fails on its own.
set -euo pipefail

root_dir="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
generator="$root_dir/tools/flight-tune-campaign/examples/alia250-xplane/generate_matrix.py"

if [ ! -f "$generator" ]; then
    echo "FORBIDDEN: no scenario matrix generator at $generator" >&2
    exit 1
fi

corpus="$(dirname "$generator")"

# Every condition artifact states the schema the trial contract accepts. A
# corpus at an older schema decodes here and fails a campaign at preparation,
# which is the failure this line moves forward.
schema=$(grep -o 'CONDITION_SET_SCHEMA_VERSION: u16 = [0-9]*' \
    "$root_dir/crates/pilotage-trial/src/limits.rs" | grep -o '[0-9]*$')
if [ -z "$schema" ]; then
    echo "FORBIDDEN: cannot read the condition schema version" >&2
    exit 1
fi

stale=$(grep -L "\"schema_version\":$schema," "$corpus"/conditions/*.json || true)
if [ -n "$stale" ]; then
    echo "FORBIDDEN: a condition artifact does not declare schema $schema:" >&2
    echo "$stale" >&2
    exit 1
fi

# The generator is the authority for the bytes. Regenerating into a scratch
# directory and comparing is what makes the corpus a derivation rather than a
# document someone maintains.
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
python3 "$generator" --out "$scratch" >/dev/null

if ! diff -ru "$scratch" "$corpus" \
    --exclude='generate_matrix.py' --exclude='README.md' --exclude='*.template.json'; then
    echo "FORBIDDEN: the checked-in scenario corpus is not the generator output" >&2
    echo "Run generate_matrix.py and commit what it writes." >&2
    exit 1
fi

count=$(find "$corpus/conditions" "$corpus/scenarios" -name '*.json' | wc -l | tr -d ' ')
echo "check-scenario-matrix-corpus: OK, $count artifacts at schema $schema"
