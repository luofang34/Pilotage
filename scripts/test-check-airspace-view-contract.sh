#!/usr/bin/env bash
# Exercise the failure paths in the AirspaceView contract guard.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture="$(mktemp -d "${TMPDIR:-/tmp}/pilotage-airspace-view.XXXXXX")"
trap 'rm -rf "$fixture"' EXIT

mkdir -p "$fixture/crates" "$fixture/docs"
cp -R "$repo_root/crates/pilotage-airspace-view" "$fixture/crates/"
cp "$repo_root/docs/airspace-view-resolution-contract.md" "$fixture/docs/"

bash "$repo_root/scripts/check-airspace-view-contract.sh" "$fixture" >/dev/null

printf '%s\n' 'pub const ENGINE: &str = "MapLibre";' \
    >> "$fixture/crates/pilotage-airspace-view/src/resolve.rs"
if bash "$repo_root/scripts/check-airspace-view-contract.sh" "$fixture" >/dev/null 2>&1; then
    echo "test failed: the guard accepted a display implementation" >&2
    exit 1
fi
cp "$repo_root/crates/pilotage-airspace-view/src/resolve.rs" \
    "$fixture/crates/pilotage-airspace-view/src/resolve.rs"

sed -i.bak '/IdentifierFromAnotherCycle/d' \
    "$fixture/crates/pilotage-airspace-view/src/model.rs"
if bash "$repo_root/scripts/check-airspace-view-contract.sh" "$fixture" >/dev/null 2>&1; then
    echo "test failed: the guard accepted a missing cycle failure" >&2
    exit 1
fi
cp "$repo_root/crates/pilotage-airspace-view/src/model.rs" \
    "$fixture/crates/pilotage-airspace-view/src/model.rs"

sed -i.bak '/DirectGeometryExtentMismatch/d' \
    "$fixture/crates/pilotage-airspace-view/src/model.rs"
if bash "$repo_root/scripts/check-airspace-view-contract.sh" "$fixture" >/dev/null 2>&1; then
    echo "test failed: the guard accepted direct geometry that can enlarge a partial subject" >&2
    exit 1
fi
cp "$repo_root/crates/pilotage-airspace-view/src/model.rs" \
    "$fixture/crates/pilotage-airspace-view/src/model.rs"

sed -i.bak '/pub struct SubjectIdentityV1/d' \
    "$fixture/crates/pilotage-airspace-view/src/model.rs"
if bash "$repo_root/scripts/check-airspace-view-contract.sh" "$fixture" >/dev/null 2>&1; then
    echo "test failed: the guard accepted a subject identifier without its cycle" >&2
    exit 1
fi
cp "$repo_root/crates/pilotage-airspace-view/src/model.rs" \
    "$fixture/crates/pilotage-airspace-view/src/model.rs"

sed -i.bak '/does not enlarge a partial subject/d' \
    "$fixture/docs/airspace-view-resolution-contract.md"
if bash "$repo_root/scripts/check-airspace-view-contract.sh" "$fixture" >/dev/null 2>&1; then
    echo "test failed: the guard accepted a missing partial-subject rule" >&2
    exit 1
fi

echo "AirspaceView contract self-test: OK"
