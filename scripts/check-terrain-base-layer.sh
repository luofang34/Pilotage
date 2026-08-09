#!/usr/bin/env bash
# Verify the cosmetic terrain build and the offline Apple style boundary.
set -euo pipefail

root="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
builder="$root/crates/pilotage-terrain-build"
client="$root/clients/apple-situation"
style="$client/Resources/SituationStyle.json"
archive="$client/Resources/SituationTerrain.mbtiles"
provenance="$client/Resources/SituationTerrain.provenance.md"
resolver="$client/App/SituationStyleResource.swift"
map_view="$client/Packages/PilotageMapLibreBinding/Sources/PilotageMapLibreBinding/SituationMapView.swift"
project="$client/project.yml"
expected_digest="4bfb229fab057719778a65ee4b68569e16839998137bd5ddb401c5c20d00eaee"
status=0

for path in "$builder/src/lib.rs" "$builder/src/tests.rs" "$style" "$archive" \
    "$provenance" "$resolver" "$map_view" "$project"; do
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
    ([.layers[] | select(.source == "pilotage-terrain" and .type == "color-relief")] | length == 0) and
    (has("terrain") | not)
' "$style" >/dev/null; then
    echo "FORBIDDEN: the situation style must use the Terrarium hillshade contract" >&2
    status=1
fi

if ! jq -e '
    .layers[] |
    select(.id == "pilotage-terrain-hillshade") |
    .paint["hillshade-shadow-color"] == "#050b11" and
    .paint["hillshade-highlight-color"] == "#526879" and
    .paint["hillshade-accent-color"] == "#1b3243"
' "$style" >/dev/null; then
    echo "FORBIDDEN: the terrain hillshade must use the dark blue-grey palette" >&2
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

if [ "${PILOTAGE_TERRAIN_SKIP_REBUILD:-0}" != "1" ]; then
    generated_archive="$(mktemp)"
    trap 'rm -f "$generated_archive"' EXIT
    cargo run --quiet \
        --manifest-path "$builder/Cargo.toml" \
        --example build_situation_fixture \
        -- "$generated_archive"
    if ! cmp -s "$archive" "$generated_archive"; then
        echo "FORBIDDEN: SituationTerrain.mbtiles is not the example output" >&2
        status=1
    fi
fi

actual_digest="$(shasum -a 256 "$archive" | awk '{print $1}')"
if [ "$actual_digest" != "$expected_digest" ]; then
    echo "FORBIDDEN: SituationTerrain.mbtiles does not match its reproducible source" >&2
    status=1
fi

if ! grep -Fq "$expected_digest" "$provenance"; then
    echo "FORBIDDEN: the terrain provenance must record the archive digest" >&2
    status=1
fi

if [ "$status" -ne 0 ]; then
    echo "Terrain base layer: FAILED" >&2
    exit 1
fi

echo "Terrain base layer: OK"
