#!/usr/bin/env bash
# Prove that the terrain boundary guard rejects unsafe delivery changes.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture="$(mktemp -d)"
trap 'rm -rf "$fixture"' EXIT

mkdir -p \
    "$fixture/scripts" \
    "$fixture/crates/pilotage-terrain-build/src" \
    "$fixture/crates/pilotage-terrain-build/examples" \
    "$fixture/clients/apple-situation/App" \
    "$fixture/clients/apple-situation/Resources" \
    "$fixture/clients/apple-situation/Packages/PilotageMapLibreBinding/Sources/PilotageMapLibreBinding"

cp "$root/scripts/check-terrain-base-layer.sh" "$fixture/scripts/"
cp "$root/crates/pilotage-terrain-build/Cargo.toml" \
    "$fixture/crates/pilotage-terrain-build/"
cp "$root/crates/pilotage-terrain-build/src/lib.rs" \
    "$root/crates/pilotage-terrain-build/src/tests.rs" \
    "$fixture/crates/pilotage-terrain-build/src/"
cp "$root/crates/pilotage-terrain-build/examples/build_situation_fixture.rs" \
    "$fixture/crates/pilotage-terrain-build/examples/"
cp "$root/clients/apple-situation/App/SituationStyleResource.swift" \
    "$fixture/clients/apple-situation/App/"
cp "$root/clients/apple-situation/Resources/SituationStyle.json" \
    "$root/clients/apple-situation/Resources/SituationTerrain.mbtiles" \
    "$root/clients/apple-situation/Resources/SituationTerrain.provenance.md" \
    "$fixture/clients/apple-situation/Resources/"
cp "$root/clients/apple-situation/Packages/PilotageMapLibreBinding/Sources/PilotageMapLibreBinding/SituationMapView.swift" \
    "$fixture/clients/apple-situation/Packages/PilotageMapLibreBinding/Sources/PilotageMapLibreBinding/"
cp "$root/clients/apple-situation/project.yml" \
    "$fixture/clients/apple-situation/"

PILOTAGE_TERRAIN_SKIP_REBUILD=1 \
    bash "$fixture/scripts/check-terrain-base-layer.sh" "$fixture" >/dev/null

sed -i.bak 's/#526879/#ff0000/' \
    "$fixture/clients/apple-situation/Resources/SituationStyle.json"
if PILOTAGE_TERRAIN_SKIP_REBUILD=1 \
    bash "$fixture/scripts/check-terrain-base-layer.sh" "$fixture" >/dev/null 2>&1; then
    echo "the terrain guard accepted an unsafe colour" >&2
    exit 1
fi

cp "$root/clients/apple-situation/Resources/SituationStyle.json" \
    "$fixture/clients/apple-situation/Resources/"
sed -i.bak 's/components.scheme = "mbtiles"/components.scheme = "file"/' \
    "$fixture/clients/apple-situation/App/SituationStyleResource.swift"
if PILOTAGE_TERRAIN_SKIP_REBUILD=1 \
    bash "$fixture/scripts/check-terrain-base-layer.sh" "$fixture" >/dev/null 2>&1; then
    echo "the terrain guard accepted a non-MBTiles resource URL" >&2
    exit 1
fi

cp "$root/clients/apple-situation/App/SituationStyleResource.swift" \
    "$fixture/clients/apple-situation/App/"
printf '\nbuild_package(source);\n' >> "$fixture/crates/pilotage-terrain-build/src/lib.rs"
if PILOTAGE_TERRAIN_SKIP_REBUILD=1 \
    bash "$fixture/scripts/check-terrain-base-layer.sh" "$fixture" >/dev/null 2>&1; then
    echo "the terrain guard accepted an SVS package path" >&2
    exit 1
fi

cp "$root/crates/pilotage-terrain-build/src/lib.rs" \
    "$fixture/crates/pilotage-terrain-build/src/"
printf '\nbuild_package(source);\n' \
    >> "$fixture/crates/pilotage-terrain-build/examples/build_situation_fixture.rs"
if PILOTAGE_TERRAIN_SKIP_REBUILD=1 \
    bash "$fixture/scripts/check-terrain-base-layer.sh" "$fixture" >/dev/null 2>&1; then
    echo "the terrain guard accepted an SVS package path in an example" >&2
    exit 1
fi

cp "$root/crates/pilotage-terrain-build/examples/build_situation_fixture.rs" \
    "$fixture/crates/pilotage-terrain-build/examples/"
sed -i.bak '/- path: Resources/d' \
    "$fixture/clients/apple-situation/project.yml"
if PILOTAGE_TERRAIN_SKIP_REBUILD=1 \
    bash "$fixture/scripts/check-terrain-base-layer.sh" "$fixture" >/dev/null 2>&1; then
    echo "the terrain guard accepted an application without the terrain resource" >&2
    exit 1
fi

cp "$root/clients/apple-situation/project.yml" \
    "$fixture/clients/apple-situation/"
printf '\npilotage-svs-db = "0.1"\n' >> "$fixture/crates/pilotage-terrain-build/Cargo.toml"
if PILOTAGE_TERRAIN_SKIP_REBUILD=1 \
    bash "$fixture/scripts/check-terrain-base-layer.sh" "$fixture" >/dev/null 2>&1; then
    echo "the terrain guard accepted an SVS database dependency" >&2
    exit 1
fi

echo "Terrain base layer guard self-test: OK"
