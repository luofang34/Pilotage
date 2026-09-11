"""Keep complete vector tiles at each geographic detail level."""
import argparse
import json
import math
from pathlib import Path
import sqlite3


def tile_rectangle(band):
    count = 2 ** band["min_zoom"]

    def row(latitude):
        latitude = min(85.0511287798066, max(-85.0511287798066, latitude))
        return (1 - math.asinh(math.tan(math.radians(latitude))) / math.pi) * count / 2

    return [
        max(0, math.floor((band["min_lon_deg"] + 180) / 360 * count + 1e-10)),
        max(0, math.floor(row(band["max_lat_deg"]) + 1e-10)),
        min(count, math.ceil((band["max_lon_deg"] + 180) / 360 * count - 1e-10)),
        min(count, math.ceil(row(band["min_lat_deg"]) - 1e-10)),
    ]


def complete_tile_plan(plan):
    result = {**plan, "bands": []}
    for band in plan["bands"]:
        left, top, right, bottom = tile_rectangle(band)
        count = 2 ** band["min_zoom"]

        def latitude(row):
            return math.degrees(math.atan(math.sinh(math.pi * (1 - 2 * row / count))))

        result["bands"].append({
            **band, "tile_rectangle": [left, top, right, bottom],
            "min_lon_deg": left / count * 360 - 180,
            "max_lon_deg": right / count * 360 - 180,
            "min_lat_deg": latitude(bottom), "max_lat_deg": latitude(top),
        })
    return result


def prune_archive(archive, plan):
    # Tile buffers can create adjacent tiles with only a small part of a polygon.
    with sqlite3.connect(archive) as connection:
        for band in plan["bands"]:
            left, top, right, bottom = band["tile_rectangle"]
            for zoom in range(band["min_zoom"], band["max_zoom"] + 1):
                scale = 2 ** (zoom - band["min_zoom"])
                count = 2 ** zoom
                connection.execute(
                    "DELETE FROM tiles WHERE zoom_level = ? AND "
                    "(tile_column < ? OR tile_column >= ? OR tile_row < ? OR tile_row >= ?)",
                    (zoom, left * scale, right * scale,
                     count - bottom * scale, count - top * scale),
                )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("plan", type=Path)
    parser.add_argument("--archive", type=Path)
    args = parser.parse_args()
    plan = json.loads(args.plan.read_text())
    if args.archive:
        prune_archive(args.archive, plan)
    else:
        print(json.dumps(complete_tile_plan(plan), indent=2))


if __name__ == "__main__":
    main()
