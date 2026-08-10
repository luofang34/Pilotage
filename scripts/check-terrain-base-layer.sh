#!/usr/bin/env bash
# Verify the cosmetic terrain build and the offline Apple style boundary.
set -euo pipefail

root="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
builder="$root/crates/pilotage-terrain-build"
client="$root/clients/apple-situation"
style="$client/Resources/SituationStyle.json"
archive="$client/Resources/SituationTerrain.mbtiles"
plan="$client/Resources/SituationTerrain.plan.json"
manifest="$client/Resources/SituationTerrain.manifest.json"
fetcher="$client/scripts/build-situation-terrain.sh"
provenance="$client/Resources/SituationTerrain.provenance.md"
resolver="$client/App/SituationStyleResource.swift"
map_view="$client/Packages/PilotageMapLibreBinding/Sources/PilotageMapLibreBinding/SituationMapView.swift"
project="$client/project.yml"
status=0

# The archive itself is a build artifact and is not committed, so it is checked only when a
# working tree holds one. The plan and the manifest are committed and are always checked:
# they are what says which tiles an archive should contain and what one build produced.
for path in "$builder/src/lib.rs" "$builder/src/tests.rs" "$style" "$plan" "$manifest" \
    "$fetcher" "$provenance" "$resolver" "$map_view" "$project"; do
    if [ ! -f "$path" ]; then
        echo "FORBIDDEN: required terrain file is missing: $path" >&2
        status=1
    fi
done

if [ "$status" -ne 0 ]; then
    exit 1
fi

if ! grep -Eq 'source: &[A-Za-z_]*SourceDataset' "$builder/src/lib.rs"; then
    echo "FORBIDDEN: the terrain build must take SourceDataset directly" >&2
    status=1
fi

if grep -RInE --include='*.rs' \
    'CandidatePackage|build_package|decode_package|verify_artifact' \
    "$builder" >/dev/null; then
    echo "FORBIDDEN: the cosmetic terrain build must not read or build an SVS-02 package" >&2
    status=1
fi

if grep -Fq 'pilotage-svs-db' "$builder/Cargo.toml"; then
    echo "FORBIDDEN: the cosmetic terrain crate must not depend on pilotage-svs-db" >&2
    status=1
fi

if ! grep -Eq 'assert_eq![(]first[.]bytes[(][)], second[.]bytes[(][)][)]' \
    "$builder/src/tests.rs"; then
    echo "FORBIDDEN: the terrain build must compare complete archive bytes" >&2
    status=1
fi

if ! jq -e '
    .sources["pilotage-terrain"].type == "raster-dem" and
    .sources["pilotage-terrain"].encoding == "terrarium" and
    .sources["pilotage-terrain"].tileSize == 256 and
    .sources["pilotage-terrain"].url == "__PILOTAGE_TERRAIN_MBTILES_URL__" and
    ([.layers[] | select(.id == "pilotage-terrain-hillshade" and .type == "hillshade" and .source == "pilotage-terrain")] | length == 1) and
    ([.layers[] | select(.id == "pilotage-terrain-relief" and .type == "color-relief" and .source == "pilotage-terrain")] | length == 1) and
    (has("terrain") | not)
' "$style" >/dev/null; then
    echo "FORBIDDEN: the situation style must use the Terrarium hillshade contract" >&2
    status=1
fi

if ! jq -e '
    .layers[] |
    select(.id == "pilotage-terrain-hillshade") |
    .paint["hillshade-shadow-color"] == "#241f16" and
    .paint["hillshade-highlight-color"] == "#cfc9b4" and
    .paint["hillshade-accent-color"] == "#3a3226"
' "$style" >/dev/null; then
    echo "FORBIDDEN: the terrain hillshade must use the warm neutral palette" >&2
    status=1
fi

# The colour ramp is what separates sea from shore and low ground from high. A ramp that
# reads the same on both sides of sea level draws a coastline that is not there, and a
# ramp that is not driven by elevation is a flat wash that says nothing about height.
relief_ramp="$(jq -c '
    .layers[] | select(.id == "pilotage-terrain-relief") | .paint["color-relief-color"]
' "$style")"
if ! printf '%s' "$relief_ramp" | jq -e '
    .[0] == "interpolate" and .[2] == ["elevation"] and
    ((. | length) >= 12)
' >/dev/null 2>&1; then
    echo "FORBIDDEN: the terrain relief must interpolate colour over elevation" >&2
    status=1
fi

# The ramp must start below sea level, so bathymetry reads as water and not as ground.
if ! printf '%s' "$relief_ramp" | jq -e '.[3] < 0' >/dev/null 2>&1; then
    echo "FORBIDDEN: the terrain relief must reach below sea level" >&2
    status=1
fi

sea_colour="$(printf '%s' "$relief_ramp" | jq -r '
    [range(3; length; 2) as $i | select(.[$i] == 0) | .[$i + 1]] | .[0] // ""
')"
below_colour="$(printf '%s' "$relief_ramp" | jq -r '
    [range(3; length; 2) as $i | select(.[$i] == -1) | .[$i + 1]] | .[0] // ""
')"
if [ -z "$sea_colour" ] || [ -z "$below_colour" ] || [ "$sea_colour" = "$below_colour" ]; then
    echo "FORBIDDEN: the terrain relief must change colour across sea level" >&2
    status=1
fi

if grep -Eqi 'https?://' "$style"; then
    echo "FORBIDDEN: the base style must have no network URL" >&2
    status=1
fi

if ! grep -Fq 'components.scheme = "mbtiles"' "$resolver" \
    || ! grep -Fq 'archiveURL.path.hasPrefix("/")' "$resolver" \
    || ! grep -Fq 'forResource: "SituationTerrain"' "$resolver"; then
    echo "FORBIDDEN: the client must resolve an absolute bundled mbtiles URL at run time" >&2
    status=1
fi

if ! grep -Fq 'MLNMapView(frame: .zero, styleJSON: styleJSON)' "$map_view"; then
    echo "FORBIDDEN: MapLibre must receive the resolved in-memory style" >&2
    status=1
fi

if ! grep -Eq '^[[:space:]]*- path: Resources[[:space:]]*$' "$project"; then
    echo "FORBIDDEN: the Apple target must include the terrain archive as a resource" >&2
    status=1
fi

if grep -Eq '^clients/apple-situation/Resources/SituationTerrain\.mbtiles$' "$root/.gitignore"; then
    :
else
    echo "FORBIDDEN: the terrain archive must stay a build artifact, not a repository file" >&2
    status=1
fi

# A world band keeps a zoomed-out map from showing empty ocean, and a regional band gives
# the zoom a pilot reads. An archive with only one of them looks correct at one zoom and
# blank at the other.
if ! jq -e '
    (.bands | length) >= 2 and
    ([.bands[] | select(.min_lon_deg <= -180 and .max_lon_deg >= 180)] | length == 1) and
    ([.bands[] | select(.max_zoom >= 10)] | length >= 1) and
    (.encoding == "terrarium") and
    (.tile_size == 256) and
    (.attribution | length > 0)
' "$plan" >/dev/null; then
    echo "FORBIDDEN: the terrain plan must cover the world and a flown region in Terrarium" >&2
    status=1
fi

plan_digest="$(shasum -a 256 "$plan" | awk '{print $1}')"
if [ "$(jq -r '.plan_sha256 // ""' "$manifest")" != "$plan_digest" ]; then
    echo "FORBIDDEN: the terrain manifest does not describe the committed plan" >&2
    status=1
fi

if ! jq -e '.tiles_written > 0 and .tiles_written == .tiles_requested' "$manifest" >/dev/null; then
    echo "FORBIDDEN: the terrain manifest must record a complete fetch" >&2
    status=1
fi

# Attribution is a licence condition of the tile source, so it has to reach the map and not
# only this repository.
attribution="$(jq -r '.attribution' "$plan")"
if ! jq -e --arg text "$attribution" \
    '.sources["pilotage-terrain"].attribution == $text' "$style" >/dev/null; then
    echo "FORBIDDEN: the style must carry the tile source attribution" >&2
    status=1
fi
if ! grep -Fq "$attribution" "$provenance"; then
    echo "FORBIDDEN: the terrain provenance must carry the tile source attribution" >&2
    status=1
fi

if [ -f "$archive" ]; then
    actual_digest="$(shasum -a 256 "$archive" | awk '{print $1}')"
    if [ "$actual_digest" != "$(jq -r '.archive_sha256 // ""' "$manifest")" ]; then
        echo "FORBIDDEN: the terrain archive does not match its manifest" >&2
        status=1
    fi
fi

if [ "$status" -ne 0 ]; then
    echo "Terrain base layer: FAILED" >&2
    exit 1
fi

echo "Terrain base layer: OK"
