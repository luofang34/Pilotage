#!/usr/bin/env bash
# Proves the corpus guard can still fail. A guard that only ever passes is a
# guard nobody has seen refuse anything.
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT

mkdir -p "$fixture/scripts" \
    "$fixture/crates/pilotage-trial/src" \
    "$fixture/tools/flight-tune-campaign/examples/alia250-xplane"
cp "$root_dir/scripts/check-scenario-matrix-corpus.sh" "$fixture/scripts/"
cp "$root_dir/crates/pilotage-trial/src/limits.rs" "$fixture/crates/pilotage-trial/src/"
corpus="$fixture/tools/flight-tune-campaign/examples/alia250-xplane"
cp "$root_dir/tools/flight-tune-campaign/examples/alia250-xplane/generate_matrix.py" "$corpus/"
python3 "$corpus/generate_matrix.py" --out "$corpus" >/dev/null

# A corpus the generator just wrote is accepted.
bash "$fixture/scripts/check-scenario-matrix-corpus.sh" "$fixture" >/dev/null

# One edited byte in one artifact is refused, which is the hand-maintained
# artifact this guard exists to catch.
edited=$(find "$corpus/conditions" -name '*.json' | LC_ALL=C sort | head -1)
python3 - "$edited" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
payload = path.read_bytes().replace(b'"seed":', b'"seed" :', 1)
path.write_bytes(payload)
PY
if bash "$fixture/scripts/check-scenario-matrix-corpus.sh" "$fixture" >/dev/null 2>&1; then
    echo "the corpus guard accepted a hand-edited artifact" >&2
    exit 1
fi
python3 "$corpus/generate_matrix.py" --out "$corpus" >/dev/null

# A version number changed by hand is refused, which is the exact repair the
# corpus rule forbids.
python3 - "$edited" <<'PY'
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
payload = path.read_bytes().replace(b'"schema_version":4', b'"schema_version":3', 1)
path.write_bytes(payload)
PY
if bash "$fixture/scripts/check-scenario-matrix-corpus.sh" "$fixture" >/dev/null 2>&1; then
    echo "the corpus guard accepted an artifact at a stale schema" >&2
    exit 1
fi
python3 "$corpus/generate_matrix.py" --out "$corpus" >/dev/null

# An artifact no cell schedules is refused, so a scenario cannot be left behind
# where nothing regenerates it and nothing runs it.
cp "$edited" "$corpus/conditions/orphan.json"
if bash "$fixture/scripts/check-scenario-matrix-corpus.sh" "$fixture" >/dev/null 2>&1; then
    echo "the corpus guard accepted an orphan artifact" >&2
    exit 1
fi
rm "$corpus/conditions/orphan.json"

# A missing artifact is refused.
rm "$edited"
if bash "$fixture/scripts/check-scenario-matrix-corpus.sh" "$fixture" >/dev/null 2>&1; then
    echo "the corpus guard accepted a missing artifact" >&2
    exit 1
fi

echo "test-check-scenario-matrix-corpus: OK"
