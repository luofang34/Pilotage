#!/usr/bin/env bash
# Prove that the terrain boundary guard rejects unsafe delivery changes.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture="$(mktemp -d)"
trap 'rm -rf "$fixture"' EXIT

mkdir -p \
    "$fixture/scripts" \
    "$fixture/clients/apple/scripts" \
    "$fixture/crates/pilotage-terrain-build/src" \
    "$fixture/crates/pilotage-terrain-build/examples" \
    "$fixture/clients/apple/App" \
    "$fixture/clients/apple/Resources" \
    "$fixture/clients/apple/Packages/PilotageMapLibreBinding/Sources/PilotageMapLibreBinding"

cp "$root/scripts/check-terrain-base-layer.sh" "$fixture/scripts/"
cp "$root/crates/pilotage-terrain-build/Cargo.toml" \
    "$fixture/crates/pilotage-terrain-build/"
cp "$root/crates/pilotage-terrain-build/src/lib.rs" \
    "$root/crates/pilotage-terrain-build/src/tests.rs" \
    "$fixture/crates/pilotage-terrain-build/src/"
cp "$root/crates/pilotage-terrain-build/examples/build_situation_fixture.rs" \
    "$fixture/crates/pilotage-terrain-build/examples/"
cp "$root/clients/apple/App/SituationStyleResource.swift" \
    "$fixture/clients/apple/App/"
cp "$root/clients/apple/Resources/SituationStyle.json" \
    "$root/clients/apple/Resources/SituationCoastline.plan.json" \
    "$root/clients/apple/Resources/SituationCoastline.manifest.json" \
    "$root/clients/apple/Resources/SituationCoastline.provenance.md" \
    "$root/clients/apple/Resources/SituationTerrain.plan.json" \
    "$root/clients/apple/Resources/SituationTerrain.manifest.json" \
    "$root/clients/apple/Resources/SituationTerrain.provenance.md" \
    "$fixture/clients/apple/Resources/"
cp "$root/clients/apple/scripts/build-situation-terrain.sh" \
    "$root/clients/apple/scripts/generate-project.sh" \
    "$fixture/clients/apple/scripts/"
cp "$root/clients/apple/scripts/build-situation-coastline.sh" \
    "$fixture/clients/apple/scripts/"
cp "$root/.gitignore" "$fixture/"
cp "$root/clients/apple/Packages/PilotageMapLibreBinding/Sources/PilotageMapLibreBinding/SituationMapView.swift" \
    "$fixture/clients/apple/Packages/PilotageMapLibreBinding/Sources/PilotageMapLibreBinding/"
cp "$root/clients/apple/project.yml" \
    "$fixture/clients/apple/"

PILOTAGE_TERRAIN_SKIP_REBUILD=1 \
    bash "$fixture/scripts/check-terrain-base-layer.sh" "$fixture" >/dev/null

# A mutation that changes nothing proves nothing. The guard then reads an
# untouched fixture, accepts it correctly, and the case reports a hole that
# is not there. Every mutation has to say what it expects to change, so a
# pattern that stops matching the file it edits fails here and not later.
mutate() {
    local file="$1"
    shift
    local before
    before="$(shasum -a 256 "$file" | awk '{print $1}')"
    sed -i.bak "$@" "$file"
    if [ "$(shasum -a 256 "$file" | awk '{print $1}')" = "$before" ]; then
        echo "the mutation changed nothing in $file: $*" >&2
        exit 1
    fi
}

mutate "$fixture/clients/apple/Resources/SituationStyle.json" 's/#cfc9b4/#ff0000/'
if PILOTAGE_TERRAIN_SKIP_REBUILD=1 \
    bash "$fixture/scripts/check-terrain-base-layer.sh" "$fixture" >/dev/null 2>&1; then
    echo "the terrain guard accepted an unsafe colour" >&2
    exit 1
fi
cp "$root/clients/apple/Resources/SituationStyle.json" \
    "$fixture/clients/apple/Resources/"

# Every mutation below must be refused; the helper restores the style
# between them.
# A plan the guard reads is also a plan its manifest describes, and the
# manifest carries the plan's digest. Rewriting the plan therefore fails the
# digest check FIRST, and every case below would be "rejected" for a reason
# that has nothing to do with the invariant it claims to prove. The manifest
# is re-synced from the mutated plan so the only thing left wrong is the
# invariant under test, and the case asserts the message it expects.
reject_plan() {
    local expected="$1"
    local plan="$fixture/clients/apple/Resources/SituationCoastline.plan.json"
    local manifest="$fixture/clients/apple/Resources/SituationCoastline.manifest.json"
    if cmp -s "$root/clients/apple/Resources/SituationCoastline.plan.json" "$plan"; then
        echo "the mutation for \"$expected\" changed nothing in the plan" >&2
        exit 1
    fi
    python3 - "$plan" "$manifest" <<'RESYNC'
import hashlib
import json
import sys

plan_path, manifest_path = sys.argv[1], sys.argv[2]
plan = json.load(open(plan_path, encoding="utf-8"))
manifest = json.load(open(manifest_path, encoding="utf-8"))
manifest["plan_sha256"] = hashlib.sha256(open(plan_path, "rb").read()).hexdigest()
for key in ("dataset_scale", "closest_zoom", "sources", "bands"):
    manifest[key] = plan[key]
json.dump(manifest, open(manifest_path, "w", encoding="utf-8"), indent=2)
open(manifest_path, "a", encoding="utf-8").write("\n")
RESYNC
    local output
    if output="$(PILOTAGE_TERRAIN_SKIP_REBUILD=1 \
        bash "$fixture/scripts/check-terrain-base-layer.sh" "$fixture" 2>&1)"; then
        echo "the terrain guard accepted $expected" >&2
        exit 1
    fi
    if ! printf '%s' "$output" | grep -Fq "$expected"; then
        echo "the terrain guard refused $expected for another reason:" >&2
        printf '%s\n' "$output" >&2
        exit 1
    fi
    cp "$root/clients/apple/Resources/SituationCoastline.plan.json" "$plan"
    cp "$root/clients/apple/Resources/SituationCoastline.manifest.json" "$manifest"
}

reject_style() {
    if PILOTAGE_TERRAIN_SKIP_REBUILD=1 \
        bash "$fixture/scripts/check-terrain-base-layer.sh" "$fixture" >/dev/null 2>&1; then
        echo "the terrain guard accepted $1" >&2
        exit 1
    fi
    cp "$root/clients/apple/Resources/SituationStyle.json" \
        "$fixture/clients/apple/Resources/"
}

# Every zoom a vector source declares must cover the world: a band that
# stops at a longitude stops the map's shapes at a straight line.
python3 - "$fixture/clients/apple/Resources/SituationCoastline.plan.json" <<'BAND_REGIONAL'
import json
import sys
path = sys.argv[1]
plan = json.load(open(path))
plan["bands"].append({
    "name": "region", "min_zoom": 8, "max_zoom": 9,
    "min_lat_deg": 45.7, "max_lat_deg": 48.1,
    "min_lon_deg": 5.8, "max_lon_deg": 10.6,
})
plan["closest_zoom"] = 9
json.dump(plan, open(path, "w"))
BAND_REGIONAL
reject_plan "every zoom the coastline declares must cover the world"

python3 - "$fixture/clients/apple/Resources/SituationCoastline.plan.json" <<'BAND_GAP'
import json
import sys
path = sys.argv[1]
plan = json.load(open(path))
plan["bands"][0]["min_zoom"] = 2
json.dump(plan, open(path, "w"))
BAND_GAP
reject_plan "every zoom the coastline declares must cover the world"

python3 - "$fixture/clients/apple/Resources/SituationCoastline.plan.json" <<'CLOSEST'
import json
import sys
path = sys.argv[1]
plan = json.load(open(path))
plan["closest_zoom"] = plan["closest_zoom"] + 1
json.dump(plan, open(path, "w"))
CLOSEST
reject_plan "every zoom the coastline declares must cover the world"


# The polygon layers are the source of the coastline. The elevation ramp is not a
# coastline source.
python3 - "$fixture/clients/apple/Resources/SituationStyle.json" <<'REMOVE_COASTLINE'
import json, sys
path = sys.argv[1]
style = json.load(open(path))
del style["sources"]["pilotage-coastline"]
style["layers"] = [
    layer for layer in style["layers"]
    if layer["id"] not in {
        "pilotage-ocean-fill",
        "pilotage-land-fill",
        "pilotage-lake-fill",
    }
]
for layer in style["layers"]:
    if layer["id"] == "pilotage-terrain-relief":
        layer["paint"]["color-relief-opacity"] = 1
        ramp = layer["paint"]["color-relief-color"]
        ramp[ramp.index(-1) + 1] = "#081d29"
json.dump(style, open(path, "w"), indent=2)
REMOVE_COASTLINE
if PILOTAGE_TERRAIN_SKIP_REBUILD=1 \
    bash "$fixture/scripts/check-terrain-base-layer.sh" "$fixture" >/dev/null 2>&1; then
    echo "the terrain guard accepted an elevation-only coastline" >&2
    exit 1
fi
cp "$root/clients/apple/Resources/SituationStyle.json" \
    "$fixture/clients/apple/Resources/"

# The ramp must not classify water at sea level.
python3 - "$fixture/clients/apple/Resources/SituationStyle.json" <<'THRESHOLD'
import json, sys
path = sys.argv[1]
style = json.load(open(path))
for layer in style["layers"]:
    if layer["id"] == "pilotage-terrain-relief":
        ramp = layer["paint"]["color-relief-color"]
        ramp[ramp.index(-1) + 1] = "#081d29"
json.dump(style, open(path, "w"), indent=2)
THRESHOLD
if PILOTAGE_TERRAIN_SKIP_REBUILD=1 \
    bash "$fixture/scripts/check-terrain-base-layer.sh" "$fixture" >/dev/null 2>&1; then
    echo "the terrain guard accepted a sea-level threshold" >&2
    exit 1
fi
cp "$root/clients/apple/Resources/SituationStyle.json" \
    "$fixture/clients/apple/Resources/"

# Height has to come from elevation. A constant colour is a flat wash that says nothing.
python3 - "$fixture/clients/apple/Resources/SituationStyle.json" <<'CONSTANT'
import json, sys
path = sys.argv[1]
style = json.load(open(path))
for layer in style["layers"]:
    if layer["id"] == "pilotage-terrain-relief":
        layer["paint"]["color-relief-color"] = "#808080"
json.dump(style, open(path, "w"), indent=2)
CONSTANT
if PILOTAGE_TERRAIN_SKIP_REBUILD=1 \
    bash "$fixture/scripts/check-terrain-base-layer.sh" "$fixture" >/dev/null 2>&1; then
    echo "the terrain guard accepted a relief that ignores elevation" >&2
    exit 1
fi

cp "$root/clients/apple/Resources/SituationStyle.json" \
    "$fixture/clients/apple/Resources/"
mutate "$fixture/clients/apple/App/SituationStyleResource.swift" 's/components.scheme = "mbtiles"/components.scheme = "file"/'
if PILOTAGE_TERRAIN_SKIP_REBUILD=1 \
    bash "$fixture/scripts/check-terrain-base-layer.sh" "$fixture" >/dev/null 2>&1; then
    echo "the terrain guard accepted a non-MBTiles resource URL" >&2
    exit 1
fi

cp "$root/clients/apple/App/SituationStyleResource.swift" \
    "$fixture/clients/apple/App/"

# A manifest that no longer describes the committed plan means the archive on disk was
# built for different tiles than the ones the repository asks for.
mutate "$fixture/clients/apple/Resources/SituationTerrain.plan.json" 's/"min_zoom": 6/"min_zoom": 7/'
if PILOTAGE_TERRAIN_SKIP_REBUILD=1 \
    bash "$fixture/scripts/check-terrain-base-layer.sh" "$fixture" >/dev/null 2>&1; then
    echo "the terrain guard accepted a manifest that does not match its plan" >&2
    exit 1
fi
cp "$root/clients/apple/Resources/SituationTerrain.plan.json" \
    "$fixture/clients/apple/Resources/"

mutate "$fixture/clients/apple/Resources/SituationCoastline.plan.json" 's/"closest_zoom": 7/"closest_zoom": 6/'
reject_plan "every zoom the coastline declares must cover the world"

# Attribution is a licence condition of the tile source and has to reach the map.
python3 - "$fixture/clients/apple/Resources/SituationStyle.json" <<'STRIP'
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
cp "$root/clients/apple/Resources/SituationStyle.json" \
    "$fixture/clients/apple/Resources/"

# A committed archive would put a large build artifact in history and hide which tiles it
# was built from.
grep -v '^clients/apple/Resources/SituationTerrain\.mbtiles$' \
    "$root/.gitignore" | \
    grep -v '^clients/apple/Resources/SituationCoastline\.mbtiles$' \
    > "$fixture/.gitignore"
if PILOTAGE_TERRAIN_SKIP_REBUILD=1 \
    bash "$fixture/scripts/check-terrain-base-layer.sh" "$fixture" >/dev/null 2>&1; then
    echo "the terrain guard accepted a committed terrain archive" >&2
    exit 1
fi
cp "$root/.gitignore" "$fixture/"

# The web renderer gets its globe from the same style file this one reads.
python3 - "$fixture/clients/apple/Resources/SituationStyle.json" <<'FLAT'
import json, sys
path = sys.argv[1]
style = json.load(open(path))
del style["projection"]
json.dump(style, open(path, "w"), indent=2)
FLAT
if PILOTAGE_TERRAIN_SKIP_REBUILD=1 \
    bash "$fixture/scripts/check-terrain-base-layer.sh" "$fixture" >/dev/null 2>&1; then
    echo "the terrain guard accepted a style with no globe projection" >&2
    exit 1
fi
cp "$root/clients/apple/Resources/SituationStyle.json" \
    "$fixture/clients/apple/Resources/"

# A map with no closest zoom stretches one elevation pixel across the screen.
mutate "$fixture/clients/apple/Packages/PilotageMapLibreBinding/Sources/PilotageMapLibreBinding/SituationMapView.swift" 's/mapView.maximumZoomLevel = maximumZoomLevel//'
if PILOTAGE_TERRAIN_SKIP_REBUILD=1 \
    bash "$fixture/scripts/check-terrain-base-layer.sh" "$fixture" >/dev/null 2>&1; then
    echo "the terrain guard accepted a map with no closest zoom" >&2
    exit 1
fi
cp "$root/clients/apple/Packages/PilotageMapLibreBinding/Sources/PilotageMapLibreBinding/SituationMapView.swift" \
    "$fixture/clients/apple/Packages/PilotageMapLibreBinding/Sources/PilotageMapLibreBinding/"

# An archive the style requires but nothing builds is a blank screen on a fresh checkout.
mutate "$fixture/clients/apple/scripts/generate-project.sh" '/build-situation-coastline.sh/d'
if PILOTAGE_TERRAIN_SKIP_REBUILD=1 \
    bash "$fixture/scripts/check-terrain-base-layer.sh" "$fixture" >/dev/null 2>&1; then
    echo "the terrain guard accepted a generator that skips an archive the style needs" >&2
    exit 1
fi
cp "$root/clients/apple/scripts/generate-project.sh" \
    "$fixture/clients/apple/scripts/"

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
mutate "$fixture/clients/apple/project.yml" '/- path: Resources/d'
if PILOTAGE_TERRAIN_SKIP_REBUILD=1 \
    bash "$fixture/scripts/check-terrain-base-layer.sh" "$fixture" >/dev/null 2>&1; then
    echo "the terrain guard accepted an application without the terrain resource" >&2
    exit 1
fi

cp "$root/clients/apple/project.yml" \
    "$fixture/clients/apple/"
printf '\npilotage-svs-db = "0.1"\n' >> "$fixture/crates/pilotage-terrain-build/Cargo.toml"
if PILOTAGE_TERRAIN_SKIP_REBUILD=1 \
    bash "$fixture/scripts/check-terrain-base-layer.sh" "$fixture" >/dev/null 2>&1; then
    echo "the terrain guard accepted an SVS database dependency" >&2
    exit 1
fi
# Every case leaves the fixture as it found it, so a case added after this
# one reads a fixture the guard accepts rather than one an earlier case
# poisoned.
cp "$root/crates/pilotage-terrain-build/Cargo.toml" \
    "$fixture/crates/pilotage-terrain-build/"

# The archive checks only run where an archive exists, and the archive is a
# build artifact no gate builds. Without this case the headline invariant of
# the coverage change has only ever executed on a machine that happened to
# have built one. A synthetic archive is enough: the check counts tiles.
archive_case() {
    local tiles="$1"
    local expected="$2"
    local archive="$fixture/clients/apple/Resources/SituationCoastline.mbtiles"
    local manifest="$fixture/clients/apple/Resources/SituationCoastline.manifest.json"
    rm -f "$archive"
    python3 - "$archive" "$tiles" <<'BUILD'
import sqlite3
import sys

archive, tiles = sys.argv[1], int(sys.argv[2])
db = sqlite3.connect(archive)
db.execute("CREATE TABLE metadata (name text, value text);")
db.execute(
    "CREATE TABLE tiles (zoom_level integer, tile_column integer,"
    " tile_row integer, tile_data blob);"
)
db.execute(
    "INSERT INTO metadata VALUES ('json', ?);",
    ('{"vector_layers":[{"id":"ocean"},{"id":"land"},{"id":"lakes"}]}',),
)
db.executemany(
    "INSERT INTO tiles VALUES (7, ?, 0, x'00');",
    ((column,) for column in range(tiles)),
)
db.commit()
db.close()
BUILD
    python3 - "$archive" "$manifest" <<'SYNC'
import hashlib
import json
import sys

archive, manifest_path = sys.argv[1], sys.argv[2]
manifest = json.load(open(manifest_path, encoding="utf-8"))
manifest["archive_sha256"] = hashlib.sha256(open(archive, "rb").read()).hexdigest()
json.dump(manifest, open(manifest_path, "w", encoding="utf-8"), indent=2)
open(manifest_path, "a", encoding="utf-8").write("\n")
SYNC
    local output
    output="$(PILOTAGE_TERRAIN_SKIP_REBUILD=1 \
        bash "$fixture/scripts/check-terrain-base-layer.sh" "$fixture" 2>&1)" && local ok=1 || local ok=0
    if [ "$expected" = "accept" ] && [ "$ok" != "1" ]; then
        echo "the terrain guard refused an archive that covers the world:" >&2
        printf '%s\n' "$output" >&2
        exit 1
    fi
    if [ "$expected" = "reject" ]; then
        if [ "$ok" = "1" ]; then
            echo "the terrain guard accepted an archive that covers a corner of the world" >&2
            exit 1
        fi
        if ! printf '%s' "$output" |
            grep -Fq "the coastline archive does not cover the world at its deepest zoom"; then
            echo "the terrain guard refused the sparse archive for another reason:" >&2
            printf '%s\n' "$output" >&2
            exit 1
        fi
    fi
    rm -f "$archive"
    cp "$root/clients/apple/Resources/SituationCoastline.manifest.json" "$manifest"
}

# Half of 4^7 is 8192. One tile short of it is a plan the build did not keep.
archive_case 8191 reject
archive_case 8192 accept
echo "Terrain base layer guard self-test: OK"
