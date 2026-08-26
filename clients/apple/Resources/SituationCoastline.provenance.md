# Situation coastline archive

This archive holds coastline data for the situation map. Coastline data means the ocean,
land, and lake polygons that classify the map surface.

## Source

The build script uses the Natural Earth 1:10m physical vectors. The committed plan gives
the source URL and the SHA-256 checksum for each source archive. The build stops if a
checksum is not correct.

Natural Earth data is in the public domain. The map carries this attribution:

> Made with Natural Earth. Free vector and raster map data at naturalearthdata.com.

## Scale

The source scale is 1:10m. This scale gives a stable coastline at the closest zoom in the
plan. It does not give survey accuracy in a harbor.

The plan uses one zoom band, and that band covers the world. A vector tile that the
archive does not hold draws nothing, and no shallower tile takes its place. A band that
stops at a longitude therefore stops the land and the sea at a straight line, and the
reader sees a rectangle of bare background. Above the deepest zoom in the plan, the
renderer stretches the tiles that it has. The picture then changes with the zoom, and
never with the position of the reader.

## Build

The archive is a build artifact. The repository does not contain the archive. Build it
from the repository root:

```text
sh clients/apple/scripts/build-situation-coastline.sh
```

The script requires GDAL with the GeoPackage and MBTiles drivers. The script caches each
verified source archive under `clients/apple/.build/coastline-sources`.

The script writes `SituationCoastline.manifest.json`. The manifest records the plan
checksum, the archive checksum, the tile count, and the source data.
