#!/usr/bin/env bash
# Verify the cosmetic terrain build and the offline Apple style boundary.
set -euo pipefail

root="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
builder="$root/crates/pilotage-terrain-build"
client="$root/clients/apple"
style="$client/Resources/SituationStyle.json"
archive="$client/Resources/SituationTerrain.mbtiles"
plan="$client/Resources/SituationTerrain.plan.json"
manifest="$client/Resources/SituationTerrain.manifest.json"
fetcher="$client/scripts/build-situation-terrain.sh"
provenance="$client/Resources/SituationTerrain.provenance.md"
coastline_archive="$client/Resources/SituationCoastline.mbtiles"
coastline_plan="$client/Resources/SituationCoastline.plan.json"
coastline_manifest="$client/Resources/SituationCoastline.manifest.json"
coastline_fetcher="$client/scripts/build-situation-coastline.sh"
coastline_provenance="$client/Resources/SituationCoastline.provenance.md"
resolver="$client/App/SituationStyleResource.swift"
map_view="$client/Packages/PilotageMapLibreBinding/Sources/PilotageMapLibreBinding/SituationMapView.swift"
project="$client/project.yml"
status=0

# The archive itself is a build artifact and is not committed, so it is checked only when a
# working tree holds one. The plan and the manifest are committed and are always checked:
# they are what says which tiles an archive should contain and what one build produced.
for path in "$builder/src/lib.rs" "$builder/src/tests.rs" "$style" "$plan" "$manifest" \
    "$fetcher" "$provenance" "$coastline_plan" "$coastline_manifest" \
    "$coastline_fetcher" "$coastline_provenance" "$resolver" "$map_view" "$project"; do
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
    .sources["pilotage-coastline"].type == "vector" and
    .sources["pilotage-coastline"].url == "__PILOTAGE_COASTLINE_MBTILES_URL__" and
    ([.layers[] | select(
        .id == "pilotage-ocean-fill" and
        .type == "fill" and
        .source == "pilotage-coastline" and
        .["source-layer"] == "ocean"
    )] | length == 1) and
    ([.layers[] | select(
        .id == "pilotage-land-fill" and
        .type == "fill" and
        .source == "pilotage-coastline" and
        .["source-layer"] == "land"
    )] | length == 1) and
    ([.layers[] | select(
        .id == "pilotage-lake-fill" and
        .type == "fill" and
        .source == "pilotage-coastline" and
        .["source-layer"] == "lakes"
    )] | length == 1) and
    ((.layers | map(.id) | index("pilotage-ocean-fill")) <
        (.layers | map(.id) | index("pilotage-terrain-relief"))) and
    ((.layers | map(.id) | index("pilotage-land-fill")) <
        (.layers | map(.id) | index("pilotage-terrain-relief"))) and
    ((.layers | map(.id) | index("pilotage-lake-fill")) <
        (.layers | map(.id) | index("pilotage-terrain-relief")))
' "$style" >/dev/null; then
    echo "FORBIDDEN: the style must draw coastline polygons below terrain relief" >&2
    status=1
fi

# A vector tile the archive does not hold draws nothing, and no shallower
# tile stands in for it the way a raster tile's parent does. So every zoom a
# vector source declares must be covered EVERYWHERE: a band that stops at a
# longitude stops the map's land and sea at a straight line, and a reader
# meets a rectangle of bare background or of flat water with no shore. Above
# the deepest declared zoom the renderer stretches every tile alike, so the
# picture changes with zoom and never with where a reader is looking.
if ! jq -e '
    (.bands | length) >= 1 and
    all(.bands[];
        .min_lon_deg <= -180 and .max_lon_deg >= 180 and
        .min_lat_deg <= -85 and .max_lat_deg >= 85) and
    (.closest_zoom == ([.bands[].max_zoom] | max)) and
    ([range(0; .closest_zoom + 1) as $zoom |
        ([.bands[] | select(.min_zoom <= $zoom and .max_zoom >= $zoom)] | length)] |
        all(. == 1))
' "$coastline_plan" >/dev/null; then
    echo "FORBIDDEN: every zoom the coastline declares must cover the world" >&2
    status=1
fi

# The archive must hold what the plan promises: a zoom that declares global
# coverage and ships a handful of tiles is a plan the build did not keep.
if [ -f "$coastline_archive" ]; then
    deepest="$(jq -r '.closest_zoom' "$coastline_plan")"
    tiles_at_deepest="$(sqlite3 "$coastline_archive" \
        "SELECT COUNT(*) FROM tiles WHERE zoom_level = $deepest;")"
    expected="$(python3 -c "print(4 ** $deepest)")"
    # An unreadable archive answers with nothing, and an empty string in an
    # arithmetic comparison is an error that the enclosing `if` would read
    # as "not less than" — a silent pass on the one input that proves least.
    if ! printf '%s' "$tiles_at_deepest" | grep -Eq '^[0-9]+$'; then
        echo "FORBIDDEN: the coastline archive did not answer for its deepest zoom" >&2
        status=1
    # A band that covers the world holds a large fraction of its zoom's
    # tiles. Half is far below what a real build produces and far above
    # what a regional band could reach.
    elif [ "$tiles_at_deepest" -lt "$((expected / 2))" ]; then
        echo "FORBIDDEN: the coastline archive does not cover the world at its deepest zoom" >&2
        status=1
    fi
fi

# The land polygon is what a reader sees through the relief, which is drawn
# at part opacity: it is what classifies ground as ground. The water
# polygons under the relief are covered by the water polygons above it and
# are there for the shoreline they give the fills beneath, so their tone is
# pinned but not their visibility.
if ! jq -e '
    ([.layers[] | select(.id == "pilotage-ocean-fill") |
        select(.paint["fill-color"] == "#061927" and .paint["fill-opacity"] == 1)] |
        length == 1) and
    ([.layers[] | select(.id == "pilotage-land-fill") |
        select(.paint["fill-color"] == "#dfe7c4" and .paint["fill-opacity"] == 1)] |
        length == 1) and
    ([.layers[] | select(.id == "pilotage-lake-fill") |
        select(.paint["fill-color"] == "#061927" and .paint["fill-opacity"] == 1)] |
        length == 1) and
    ([.layers[] | select(.id == "pilotage-terrain-relief") |
        select(.paint["color-relief-opacity"] > 0 and
            .paint["color-relief-opacity"] < 1)] | length == 1)
' "$style" >/dev/null; then
    echo "FORBIDDEN: the land polygon must remain visible below terrain relief" >&2
    status=1
fi

# Water reads as water. An elevation ramp cannot distinguish a lake at
# 500 m from ground at 500 m, so the polygon that carries that information
# draws OVER the relief and the hillshade and keeps its own colour. Below
# either, water takes the tint of the height below it and a reader meets
# green where the chart says open water.
#
# The two tones are pinned exactly and they differ, which is what makes
# open water and inland water read apart. A separate inequality assertion
# would be a rule nothing can break on its own.
if ! jq -e '
    ([.layers[] | select(
        .id == "pilotage-ocean-water" and .type == "fill" and
        .source == "pilotage-coastline" and .["source-layer"] == "ocean" and
        .paint["fill-color"] == "#e1eef5" and .paint["fill-opacity"] == 1
    )] | length == 1) and
    ([.layers[] | select(
        .id == "pilotage-lake-water" and .type == "fill" and
        .source == "pilotage-coastline" and .["source-layer"] == "lakes" and
        .paint["fill-color"] == "#93cde5" and .paint["fill-opacity"] == 1
    )] | length == 1) and
    ([.layers[] | select(
        .id == "pilotage-river" and .type == "line" and
        .source == "pilotage-coastline" and .["source-layer"] == "rivers" and
        .paint["line-color"] == "#93cde5"
    )] | length == 1) and
    (.layers | map(.id)) as $order |
    (["pilotage-ocean-water", "pilotage-lake-water", "pilotage-river"] | all(. as $id |
        ($order | index($id)) != null and
        ($order | index($id)) > ($order | index("pilotage-terrain-hillshade"))))
' "$style" >/dev/null; then
    echo "FORBIDDEN: hydrography must draw over the relief" >&2
    status=1
fi

# Drainage thins with zoom. Without the rank the whole world's drainage
# draws as a web of hairlines at the zooms the world band covers.
if ! jq -e '
    [.layers[] | select(.id == "pilotage-river") | .paint["line-width"] |
        .. | select(type == "string" and . == "scalerank")] | length > 0
' "$style" >/dev/null; then
    echo "FORBIDDEN: river width must thin by the rank the data carries" >&2
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

# The colour ramp shows elevation and depth. The polygons classify land and water.
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
if [ -z "$sea_colour" ] || [ -z "$below_colour" ] || [ "$sea_colour" != "$below_colour" ]; then
    echo "FORBIDDEN: the elevation ramp must not classify water at sea level" >&2
    status=1
fi

# One style file drives both renderers. The web renderer draws a globe from this key and
# this one ignores it, so the two stay on the same data and the same colours.
if ! jq -e '.projection.type == "globe"' "$style" >/dev/null; then
    echo "FORBIDDEN: the style must declare the globe projection for the web renderer" >&2
    status=1
fi

# A raster-dem source keeps drawing past its deepest tile by stretching what it has. The
# map has to stop near where the archive stops, and the limit has to follow the plan
# rather than be written twice.
if ! grep -Fq 'SituationTerrain.manifest' "$resolver" \
    || ! grep -Fq 'maximumZoomLevel' "$resolver"; then
    echo "FORBIDDEN: the closest zoom must be read from the terrain manifest" >&2
    status=1
fi
if ! grep -Fq 'mapView.maximumZoomLevel = maximumZoomLevel' "$map_view"; then
    echo "FORBIDDEN: the map must apply a closest zoom" >&2
    status=1
fi

if grep -Eqi 'https?://' "$style"; then
    echo "FORBIDDEN: the base style must have no network URL" >&2
    status=1
fi

if ! grep -Fq 'components.scheme = "mbtiles"' "$resolver" \
    || ! grep -Fq 'archiveURL.path.hasPrefix("/")' "$resolver" \
    || ! grep -Fq 'forResource: "SituationCoastline"' "$resolver" \
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

if grep -Eq '^clients/apple/Resources/SituationTerrain\.mbtiles$' "$root/.gitignore" \
    && grep -Eq '^clients/apple/Resources/SituationCoastline\.mbtiles$' "$root/.gitignore"; then
    :
else
    echo "FORBIDDEN: map archives must stay build artifacts" >&2
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

if ! jq -e '
    .dataset_scale == "1:10m" and
    .closest_zoom == ([.bands[].max_zoom] | max) and
    ([.sources[] | .layer // .name] | unique) == ["lakes", "land", "ocean", "rivers"] and
    all(.sources[];
        (.url | startswith("https://naturalearth.s3.amazonaws.com/")) and
        (.sha256 | test("^[0-9a-f]{64}$")) and
        (.shapefile | endswith(".shp"))) and
    ([.bands[] | select(.min_zoom == 0 and .min_lon_deg <= -180 and
        .max_lon_deg >= 180)] | length == 1) and
    ([range(0; .closest_zoom + 1) as $zoom |
        ([.bands[] | select(.min_zoom <= $zoom and .max_zoom >= $zoom)] | length)] |
        all(. == 1)) and
    (.attribution | length > 0)
' "$coastline_plan" >/dev/null; then
    echo "FORBIDDEN: the coastline plan must define verified data for each map zoom" >&2
    status=1
fi

# Several sources can feed one layer. GDAL writes the appended features
# into whatever fields the layer was created with, silently, so a source
# that names fewer fields than the one before it deletes the missing field
# from every feature in the layer. The rank the river ladder reads is such
# a field: without it the ladder's own fallback is never reached, the width
# property falls back to the renderer's default of 1, and the whole world's
# drainage draws as hairlines at every zoom.
if ! jq -e '
    [.sources[] | {layer: (.layer // .name),
                   select: (.select // "featurecla"),
                   geometry: (.geometry_type // "MULTIPOLYGON")}]
    | group_by(.layer)
    | all(.[0] as $first | all(.[]; .select == $first.select and
                                     .geometry == $first.geometry))
' "$coastline_plan" >/dev/null; then
    echo "FORBIDDEN: sources feeding one layer must agree on fields and geometry" >&2
    status=1
fi

if ! grep -Fq 'GDAL_NUM_THREADS=1 ogr2ogr -f MBTiles' "$coastline_fetcher" \
    || ! grep -Fq 'shasum -a 256' "$coastline_fetcher" \
    || ! grep -Fq "CONF=\$work/layers.json" "$coastline_fetcher"; then
    echo "FORBIDDEN: the coastline builder must verify and pack the source polygons" >&2
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

coastline_plan_digest="$(shasum -a 256 "$coastline_plan" | awk '{print $1}')"
if [ "$(jq -r '.plan_sha256 // ""' "$coastline_manifest")" != "$coastline_plan_digest" ]; then
    echo "FORBIDDEN: the coastline manifest does not describe the committed plan" >&2
    status=1
fi

if ! jq -e --slurpfile plan "$coastline_plan" '
    .tiles_written > 0 and
    .dataset_scale == $plan[0].dataset_scale and
    .closest_zoom == $plan[0].closest_zoom and
    .sources == $plan[0].sources and
    .bands == $plan[0].bands
' "$coastline_manifest" >/dev/null; then
    echo "FORBIDDEN: the coastline manifest must record a complete build" >&2
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

coastline_attribution="$(jq -r '.attribution' "$coastline_plan")"
if ! jq -e --arg text "$coastline_attribution" \
    '.sources["pilotage-coastline"].attribution == $text' "$style" >/dev/null; then
    echo "FORBIDDEN: the style must carry the coastline attribution" >&2
    status=1
fi
if ! grep -Fq "$coastline_attribution" "$coastline_provenance"; then
    echo "FORBIDDEN: the coastline provenance must carry the attribution" >&2
    status=1
fi

if [ -f "$archive" ]; then
    actual_digest="$(shasum -a 256 "$archive" | awk '{print $1}')"
    if [ "$actual_digest" != "$(jq -r '.archive_sha256 // ""' "$manifest")" ]; then
        echo "FORBIDDEN: the terrain archive does not match its manifest" >&2
        status=1
    fi
fi

if [ -f "$coastline_archive" ]; then
    actual_digest="$(shasum -a 256 "$coastline_archive" | awk '{print $1}')"
    if [ "$actual_digest" != "$(jq -r '.archive_sha256 // ""' "$coastline_manifest")" ]; then
        echo "FORBIDDEN: the coastline archive does not match its manifest" >&2
        status=1
    fi
    if ! sqlite3 "$coastline_archive" \
        "SELECT value FROM metadata WHERE name = 'json';" | jq -e '
            [.vector_layers[].id] | sort == ["lakes", "land", "ocean", "rivers"]
        ' >/dev/null; then
        echo "FORBIDDEN: the coastline archive must contain ocean, land, lakes, and rivers" >&2
        status=1
    fi
fi

# Both archives are build artifacts the style refuses to resolve without. A generator that
# builds one and not the other leaves a fresh checkout with a blank screen rather than a
# map, and the failure appears only on a machine that has never built before.
generator="$client/scripts/generate-project.sh"
for builder in build-situation-terrain.sh build-situation-coastline.sh; do
    if ! grep -Fq "$builder" "$generator"; then
        echo "FORBIDDEN: project generation must build $builder" >&2
        status=1
    fi
done

if [ "$status" -ne 0 ]; then
    echo "Terrain base layer: FAILED" >&2
    exit 1
fi

echo "Terrain base layer: OK"
