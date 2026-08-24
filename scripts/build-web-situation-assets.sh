#!/usr/bin/env bash
# Export the situation map assets for the web client.
#
# One style file drives the Apple renderer and the web renderer. The Apple
# client reads the two MBTiles archives directly. MapLibre GL JS cannot read
# an MBTiles archive, so this script exports each archive into a static
# z/x/y tile tree, writes one TileJSON document per source, and copies the
# style, the terrain manifest, and the glyph fonts beside them. The output
# directory is a build artifact and is not committed.
#
# The web client substitutes the style's three __PILOTAGE_*__ URL tokens at
# run time, exactly as the Apple client does. This script copies the style
# verbatim and never edits it.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
resources="$root/clients/apple/Resources"
out="$root/clients/web/situation-assets"

while [ $# -gt 0 ]; do
    case "$1" in
        --resources)
            resources="$2"
            shift 2
            ;;
        --out)
            out="$2"
            shift 2
            ;;
        *)
            echo "usage: $0 [--resources DIR] [--out DIR]" >&2
            exit 2
            ;;
    esac
done

style="$resources/SituationStyle.json"
terrain_manifest="$resources/SituationTerrain.manifest.json"
coastline_archive="$resources/SituationCoastline.mbtiles"
terrain_archive="$resources/SituationTerrain.mbtiles"
fonts="$resources/Fonts"

for path in "$style" "$terrain_manifest" "$coastline_archive" "$terrain_archive"; do
    if [ ! -f "$path" ]; then
        echo "MISSING: $path" >&2
        echo "Build the archives first: clients/apple/scripts/build-situation-coastline.sh" >&2
        echo "and clients/apple/scripts/build-situation-terrain.sh" >&2
        exit 1
    fi
done

# Refuse to replace a directory this script did not produce.
if [ -e "$out" ]; then
    if [ ! -f "$out/assets-manifest.json" ]; then
        echo "REFUSED: $out exists and is not a situation-assets export" >&2
        exit 1
    fi
    rm -rf "$out"
fi

python3 - "$style" "$terrain_manifest" "$coastline_archive" "$terrain_archive" \
    "$fonts" "$out" <<'EOF'
import gzip
import json
import shutil
import sqlite3
import sys
from pathlib import Path

GLYPHS_TOKEN = "__PILOTAGE_GLYPHS_URL__"
COASTLINE_TOKEN = "__PILOTAGE_COASTLINE_MBTILES_URL__"
TERRAIN_TOKEN = "__PILOTAGE_TERRAIN_MBTILES_URL__"

(
    style_path,
    terrain_manifest_path,
    coastline_archive_path,
    terrain_archive_path,
    fonts_path,
    out_path,
) = (Path(argument) for argument in sys.argv[1:7])


def fail(message: str) -> None:
    print(f"INVALID: {message}", file=sys.stderr)
    sys.exit(1)


with open(style_path, encoding="utf-8") as handle:
    style = json.load(handle)

sources = style.get("sources")
if not isinstance(sources, dict):
    fail("the style has no sources object")
coastline_source = sources.get("pilotage-coastline", {})
terrain_source = sources.get("pilotage-terrain", {})
if coastline_source.get("url") != COASTLINE_TOKEN:
    fail("the style coastline source does not carry the archive token")
if terrain_source.get("url") != TERRAIN_TOKEN:
    fail("the style terrain source does not carry the archive token")
if style.get("glyphs") != f"{GLYPHS_TOKEN}/{{fontstack}}/{{range}}.pbf":
    fail("the style does not carry the glyphs token")


def export_tiles(archive: Path, target: Path, extension: str, gunzip: bool):
    """Export an MBTiles tile table into a z/x/y tree.

    MBTiles stores rows in TMS order; the web tile URL scheme counts rows
    from the north, so each row index is flipped. Vector tiles are stored
    gzip-compressed and a static file server sends no Content-Encoding
    header, so they are decompressed on export.
    """
    connection = sqlite3.connect(f"file:{archive}?mode=ro", uri=True)
    try:
        rows = connection.execute(
            "SELECT zoom_level, tile_column, tile_row, tile_data FROM tiles"
        )
        count = 0
        min_zoom = None
        max_zoom = None
        for zoom, column, row, data in rows:
            y = (1 << zoom) - 1 - row
            tile_path = target / str(zoom) / str(column) / f"{y}.{extension}"
            tile_path.parent.mkdir(parents=True, exist_ok=True)
            payload = bytes(data)
            if gunzip and payload[:2] == b"\x1f\x8b":
                payload = gzip.decompress(payload)
            tile_path.write_bytes(payload)
            count += 1
            min_zoom = zoom if min_zoom is None else min(min_zoom, zoom)
            max_zoom = zoom if max_zoom is None else max(max_zoom, zoom)
    finally:
        connection.close()
    if count == 0:
        fail(f"{archive} holds no tiles")
    return count, min_zoom, max_zoom


out_path.mkdir(parents=True, exist_ok=True)

coastline_count, coastline_min, coastline_max = export_tiles(
    coastline_archive_path, out_path / "coastline", "pbf", gunzip=True
)
terrain_count, terrain_min, terrain_max = export_tiles(
    terrain_archive_path, out_path / "terrain", "png", gunzip=False
)


def write_tilejson(name: str, template: str, min_zoom: int, max_zoom: int) -> None:
    document = {
        "tilejson": "3.0.0",
        "name": name,
        "tiles": [template],
        "minzoom": min_zoom,
        "maxzoom": max_zoom,
    }
    with open(out_path / f"{name}.tilejson.json", "w", encoding="utf-8") as handle:
        json.dump(document, handle, indent=2, sort_keys=True)
        handle.write("\n")


write_tilejson("coastline", "coastline/{z}/{x}/{y}.pbf", coastline_min, coastline_max)
write_tilejson("terrain", "terrain/{z}/{x}/{y}.png", terrain_min, terrain_max)

shutil.copyfile(style_path, out_path / "SituationStyle.json")
shutil.copyfile(terrain_manifest_path, out_path / "SituationTerrain.manifest.json")

glyphs = None
if fonts_path.is_dir():
    shutil.copytree(fonts_path, out_path / "fonts")
    glyphs = "fonts"

manifest = {
    "schema_version": 1,
    "style": "SituationStyle.json",
    "terrain_manifest": "SituationTerrain.manifest.json",
    "sources": {
        "pilotage-coastline": "coastline.tilejson.json",
        "pilotage-terrain": "terrain.tilejson.json",
    },
    "glyphs": glyphs,
    "coastline_tiles": coastline_count,
    "terrain_tiles": terrain_count,
}
with open(out_path / "assets-manifest.json", "w", encoding="utf-8") as handle:
    json.dump(manifest, handle, indent=2, sort_keys=True)
    handle.write("\n")

print(
    f"Exported {coastline_count} coastline tiles and {terrain_count} terrain tiles"
    f" into {out_path}"
)
EOF
