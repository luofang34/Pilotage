#!/bin/sh
# Fetch coastline polygons and pack the offline vector archive.
set -eu

client_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
plan="$client_root/Resources/SituationCoastline.plan.json"
archive="$client_root/Resources/SituationCoastline.mbtiles"
manifest="$client_root/Resources/SituationCoastline.manifest.json"
force=${1:-}

for tool in curl jq shasum unzip ogr2ogr sqlite3 awk; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "$tool is required to build the coastline archive" >&2
        exit 2
    fi
done

plan_digest=$(shasum -a 256 "$plan" | awk '{print $1}')
if [ "$force" != "--force" ] && [ -f "$archive" ] && [ -f "$manifest" ] &&
    [ "$(jq -r '.plan_sha256 // ""' "$manifest")" = "$plan_digest" ] &&
    [ "$(shasum -a 256 "$archive" | awk '{print $1}')" = "$(jq -r '.archive_sha256 // ""' "$manifest")" ]; then
    echo "coastline archive is ready for the current plan"
    exit 0
fi

work=$(mktemp -d "${TMPDIR:-/tmp}/situation-coastline.XXXXXX")
trap 'rm -rf "$work"' EXIT
source_cache="$client_root/.build/coastline-sources"
mkdir -p "$source_cache"

jq -r '.sources[] | [.name, .version, .url, .sha256, .shapefile] | @tsv' "$plan" |
    while IFS="$(printf '\t')" read -r name version url expected_digest shapefile; do
        source_archive="$source_cache/$name-$version.zip"
        actual_digest=""
        if [ -f "$source_archive" ]; then
            actual_digest=$(shasum -a 256 "$source_archive" | awk '{print $1}')
        fi
        if [ "$actual_digest" != "$expected_digest" ]; then
            fetched="$work/$name.zip"
            curl --fail --location --silent --show-error --retry 3 \
                --output "$fetched" "$url"
            actual_digest=$(shasum -a 256 "$fetched" | awk '{print $1}')
            if [ "$actual_digest" != "$expected_digest" ]; then
                echo "the $name source checksum is not correct" >&2
                exit 2
            fi
            mv "$fetched" "$source_archive"
        fi
        source_dir="$work/$name"
        mkdir -p "$source_dir"
        unzip -q "$source_archive" -d "$source_dir"
        if [ ! -f "$source_dir/$shapefile" ]; then
            echo "the $name source has no $shapefile file" >&2
            exit 2
        fi
    done

geopackage="$work/coastline.gpkg"
jq -r '.sources[].name' "$plan" |
    while read -r source_name; do
        shapefile=$(jq -r --arg name "$source_name" \
            '.sources[] | select(.name == $name) | .shapefile' "$plan")
        jq -r '.bands[] | [.name, .min_zoom, .max_zoom, .min_lat_deg, .max_lat_deg, .min_lon_deg, .max_lon_deg] | @tsv' "$plan" |
            while IFS="$(printf '\t')" read -r band_name _min_zoom _max_zoom min_lat max_lat min_lon max_lon; do
                layer_name="${source_name}_${band_name}"
                if [ -f "$geopackage" ]; then
                    ogr2ogr -update -nln "$layer_name" \
                        -clipsrc "$min_lon" "$min_lat" "$max_lon" "$max_lat" \
                        -select featurecla -nlt MULTIPOLYGON -dim XY -makevalid \
                        "$geopackage" "$work/$source_name/$shapefile"
                else
                    ogr2ogr -f GPKG -nln "$layer_name" \
                        -clipsrc "$min_lon" "$min_lat" "$max_lon" "$max_lat" \
                        -select featurecla -nlt MULTIPOLYGON -dim XY -makevalid \
                        "$geopackage" "$work/$source_name/$shapefile"
                fi
            done
    done

jq '
    [
        .sources[] as $source |
        .bands[] as $band |
        {
            key: ($source.name + "_" + $band.name),
            value: {
                target_name: $source.name,
                description: ($source.name + " polygons"),
                minzoom: $band.min_zoom,
                maxzoom: $band.max_zoom
            }
        }
    ] | from_entries
' "$plan" > "$work/layers.json"

minimum_zoom=$(jq -r '[.bands[].min_zoom] | min' "$plan")
maximum_zoom=$(jq -r '[.bands[].max_zoom] | max' "$plan")
GDAL_NUM_THREADS=1 ogr2ogr -f MBTiles \
    -dsco "NAME=Pilotage situation coastline" \
    -dsco "DESCRIPTION=Natural Earth 1:10m coastline polygons" \
    -dsco "MINZOOM=$minimum_zoom" \
    -dsco "MAXZOOM=$maximum_zoom" \
    -dsco "BOUNDS=-180,-85.0511287798066,180,85.0511287798066" \
    -dsco "CONF=$work/layers.json" \
    "$work/archive.mbtiles" "$geopackage"

tiles_written=$(sqlite3 "$work/archive.mbtiles" 'SELECT COUNT(*) FROM tiles;')
if [ "$tiles_written" -eq 0 ]; then
    echo "the coastline archive has no tiles" >&2
    exit 2
fi

mv "$work/archive.mbtiles" "$archive"
archive_digest=$(shasum -a 256 "$archive" | awk '{print $1}')

jq -n \
    --arg plan_sha256 "$plan_digest" \
    --arg archive_sha256 "$archive_digest" \
    --argjson tiles_written "$tiles_written" \
    --argjson bytes "$(wc -c < "$archive" | tr -d ' ')" \
    --slurpfile plan "$plan" '
    {
        plan_sha256: $plan_sha256,
        archive_sha256: $archive_sha256,
        tiles_written: $tiles_written,
        archive_bytes: $bytes,
        source_name: $plan[0].source_name,
        dataset_scale: $plan[0].dataset_scale,
        attribution: $plan[0].attribution,
        closest_zoom: $plan[0].closest_zoom,
        sources: $plan[0].sources,
        bands: $plan[0].bands
    }
' > "$manifest"

echo "packed $tiles_written coastline tiles into $archive"
