#!/bin/sh
# Fetch published elevation tiles and pack the offline terrain archive.
#
# The archive is a build artifact and not a repository file. It is large, and its contents
# come from a published tile service rather than from this repository. What is committed is
# the plan that selects the tiles and the manifest that records what one run produced, so a
# second run can be compared with the first.
#
# The plan carries two bands. The world band covers the whole globe at low zoom so a
# zoomed-out map is never empty ocean. The regional band covers where the aircraft flies at
# the zoom a pilot reads. A raster-dem source overzooms past its highest zoom, so the
# regional band does not need to reach the closest zoom the map allows.
#
# Usage: build-situation-terrain.sh [--force]
set -eu

client_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
plan="$client_root/Resources/SituationTerrain.plan.json"
archive="$client_root/Resources/SituationTerrain.mbtiles"
manifest="$client_root/Resources/SituationTerrain.manifest.json"
force=${1:-}

for tool in curl jq sqlite3 awk shasum; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "$tool is required to build the terrain archive" >&2
        exit 2
    fi
done

plan_digest=$(shasum -a 256 "$plan" | awk '{print $1}')
if [ "$force" != "--force" ] && [ -f "$archive" ] && [ -f "$manifest" ] &&
    [ "$(jq -r '.plan_sha256 // ""' "$manifest")" = "$plan_digest" ] &&
    [ "$(shasum -a 256 "$archive" | awk '{print $1}')" = "$(jq -r '.archive_sha256 // ""' "$manifest")" ]; then
    echo "terrain archive is ready for the current plan"
    exit 0
fi

source_template=$(jq -r '.source' "$plan")
work=$(mktemp -d "${TMPDIR:-/tmp}/situation-terrain.XXXXXX")
trap 'rm -rf "$work"' EXIT
# Tiles are cached outside the working directory. A failed or interrupted run otherwise
# discards every tile it fetched, and the next run downloads the whole plan again.
tiles="$client_root/.build/terrain-tiles"
mkdir -p "$tiles"

# One line for each tile: zoom, column, XYZ row. The MBTiles row is flipped when the tile
# is inserted, because MBTiles counts rows from the south and a tile service counts from
# the north.
jq -r '.bands[] | "\(.min_zoom) \(.max_zoom) \(.min_lat_deg) \(.max_lat_deg) \(.min_lon_deg) \(.max_lon_deg)"' "$plan" |
    awk '
    function lon_to_x(lon, n) { return int((lon + 180.0) / 360.0 * n) }
    function lat_to_y(lat, n,   r, merc) {
        r = lat * 3.14159265358979323846 / 180.0
        merc = log((sin(r) + 1.0) / cos(r))
        return int((1.0 - merc / 3.14159265358979323846) / 2.0 * n)
    }
    {
        for (z = $1; z <= $2; z++) {
            n = 2 ^ z
            x0 = lon_to_x($5, n); x1 = lon_to_x($6, n)
            y0 = lat_to_y($4, n); y1 = lat_to_y($3, n)
            if (x0 < 0) x0 = 0
            if (y0 < 0) y0 = 0
            if (x1 > n - 1) x1 = n - 1
            if (y1 > n - 1) y1 = n - 1
            for (y = y0; y <= y1; y++)
                for (x = x0; x <= x1; x++)
                    print z, x, y
        }
    }' | sort -u -k1,1n -k2,2n -k3,3n > "$work/addresses"

requested=$(wc -l < "$work/addresses" | tr -d ' ')
echo "terrain tiles requested: $requested"

# A tile that fails after its retries is left out rather than failing the run. A gap at one
# address is a hole in shading; a run that stops leaves no archive at all.
awk -v template="$source_template" -v dir="$tiles" '
    {
        url = template
        gsub(/\{z\}/, $1, url)
        gsub(/\{x\}/, $2, url)
        gsub(/\{y\}/, $3, url)
        print url, dir "/" $1 "-" $2 "-" $3 ".png"
    }' "$work/addresses" |
    while read -r url path; do
        [ -s "$path" ] || printf '%s %s\n' "$url" "$path"
    done > "$work/fetches"
echo "terrain tiles to fetch: $(wc -l < "$work/fetches" | tr -d ' ')"

# shellcheck disable=SC2016
xargs -P 16 -n 2 sh -c \
    'curl --fail --silent --show-error --retry 3 --max-time 60 --output "$1" "$0" || rm -f "$1"' \
    < "$work/fetches" || true

printf 'PRAGMA journal_mode=OFF;\nPRAGMA synchronous=OFF;\nBEGIN;\n' > "$work/pack.sql"
cat >> "$work/pack.sql" <<'SCHEMA'
CREATE TABLE metadata (
        name TEXT PRIMARY KEY NOT NULL,
        value TEXT NOT NULL
    ) WITHOUT ROWID;
CREATE TABLE tiles (
        zoom_level INTEGER NOT NULL,
        tile_column INTEGER NOT NULL,
        tile_row INTEGER NOT NULL,
        tile_data BLOB NOT NULL,
        PRIMARY KEY (zoom_level, tile_column, tile_row)
    ) WITHOUT ROWID;
SCHEMA

minimum_zoom=$(jq -r '[.bands[].min_zoom] | min' "$plan")
maximum_zoom=$(jq -r '[.bands[].max_zoom] | max' "$plan")
encoding=$(jq -r '.encoding' "$plan")
tile_size=$(jq -r '.tile_size' "$plan")
attribution=$(jq -r '.attribution' "$plan")

# SQL quotes a literal by doubling the apostrophes inside it.
metadata_row() {
    value=$(printf '%s' "$2" | sed "s/'/''/g")
    printf "INSERT INTO metadata VALUES ('%s', '%s');\n" "$1" "$value" >> "$work/pack.sql"
}
metadata_row name "Pilotage situation terrain"
metadata_row type baselayer
metadata_row version 1
metadata_row format png
metadata_row encoding "$encoding"
metadata_row tile_size "$tile_size"
metadata_row minzoom "$minimum_zoom"
metadata_row maxzoom "$maximum_zoom"
metadata_row attribution "$attribution"
metadata_row description "Terrarium elevation packed by build-situation-terrain.sh"

written=0
while read -r zoom column row; do
    file="$tiles/$zoom-$column-$row.png"
    [ -s "$file" ] || continue
    tms_row=$(awk -v z="$zoom" -v y="$row" 'BEGIN { printf "%d", 2 ^ z - 1 - y }')
    printf "INSERT INTO tiles VALUES (%s, %s, %s, readfile('%s'));\n" \
        "$zoom" "$column" "$tms_row" "$file" >> "$work/pack.sql"
    written=$((written + 1))
done < "$work/addresses"
printf 'COMMIT;\n' >> "$work/pack.sql"

if [ "$written" -eq 0 ]; then
    echo "no terrain tile was fetched" >&2
    exit 2
fi

rm -f "$work/archive.mbtiles"
sqlite3 "$work/archive.mbtiles" < "$work/pack.sql" > /dev/null
mv "$work/archive.mbtiles" "$archive"

archive_digest=$(shasum -a 256 "$archive" | awk '{print $1}')
jq -n \
    --arg plan_sha256 "$plan_digest" \
    --arg archive_sha256 "$archive_digest" \
    --argjson requested "$requested" \
    --argjson written "$written" \
    --argjson bytes "$(wc -c < "$archive" | tr -d ' ')" \
    --slurpfile plan "$plan" '
    {
        plan_sha256: $plan_sha256,
        archive_sha256: $archive_sha256,
        tiles_requested: $requested,
        tiles_written: $written,
        archive_bytes: $bytes,
        source: $plan[0].source,
        source_name: $plan[0].source_name,
        attribution: $plan[0].attribution,
        bands: $plan[0].bands
    }' > "$manifest"

echo "packed $written of $requested terrain tiles into $archive"
