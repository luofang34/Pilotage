#!/usr/bin/env bash
# Prove that the terrain boundary guard rejects unsafe delivery changes.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture="$(mktemp -d)"
trap 'rm -rf "$fixture"' EXIT

mkdir -p \
    "$fixture/scripts" \
    "$fixture/clients/apple-situation/scripts" \
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
    "$root/clients/apple-situation/Resources/SituationTerrain.plan.json" \
    "$root/clients/apple-situation/Resources/SituationTerrain.manifest.json" \
    "$root/clients/apple-situation/Resources/SituationTerrain.provenance.md" \
    "$fixture/clients/apple-situation/Resources/"
cp "$root/clients/apple-situation/scripts/build-situation-terrain.sh" \
    "$fixture/clients/apple-situation/scripts/"
cp "$root/.gitignore" "$fixture/"
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

# A manifest that no longer describes the committed plan means the archive on disk was
# built for different tiles than the ones the repository asks for.
sed -i.bak 's/"min_zoom": 6/"min_zoom": 7/' \
    "$fixture/clients/apple-situation/Resources/SituationTerrain.plan.json"
if PILOTAGE_TERRAIN_SKIP_REBUILD=1 \
    bash "$fixture/scripts/check-terrain-base-layer.sh" "$fixture" >/dev/null 2>&1; then
    echo "the terrain guard accepted a manifest that does not match its plan" >&2
    exit 1
fi
cp "$root/clients/apple-situation/Resources/SituationTerrain.plan.json" \
    "$fixture/clients/apple-situation/Resources/"

# Attribution is a licence condition of the tile source and has to reach the map.
python3 - "$fixture/clients/apple-situation/Resources/SituationStyle.json" <<'STRIP'
import json, sys
path = sys.argv[1]
style = json.load(open(path))
del style["sources"]["pilotage-terrain"]["attribution"]
json.dump(style, open(path, "w"), indent=2)
STRIP
if PILOTAGE_TERRAIN_SKIP_REBUILD=1 \
    bash "$fixture/scripts/check-terrain-base-layer.sh" "$fixture" >/dev/null 2>&1; then
    echo "the terrain guard accepted a style with no source attribution" >&2
    exit 1
fi
cp "$root/clients/apple-situation/Resources/SituationStyle.json" \
    "$fixture/clients/apple-situation/Resources/"

# A committed archive would put a large build artifact in history and hide which tiles it
# was built from.
grep -v '^clients/apple-situation/Resources/SituationTerrain\.mbtiles$' \
    "$root/.gitignore" > "$fixture/.gitignore"
if PILOTAGE_TERRAIN_SKIP_REBUILD=1 \
    bash "$fixture/scripts/check-terrain-base-layer.sh" "$fixture" >/dev/null 2>&1; then
    echo "the terrain guard accepted a committed terrain archive" >&2
    exit 1
fi
cp "$root/.gitignore" "$fixture/"

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
