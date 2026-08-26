# Situation coastline archive

This archive holds the surface data for the situation map: the ocean, land, and lake
polygons that classify the map surface, and the river lines that draw its drainage.

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

Each source in the plan declares the geometry it carries and the attributes it keeps. The
river sources carry lines; the others carry polygons. GDAL writes a line source into a
polygon layer with only a warning, and that layer then draws nothing.

More than one source can feed one layer. The global 1:10m files give the world its shape.
The regional files for Europe and North America give the flown areas the drainage density
of a chart. The rank of a feature is a drawing scale, and not a name for the file that
holds it: the global file holds mostly rank 0 to 9 and some dozens of features above it,
and the regional files hold rank 10 to 12 only. The style reads the rank.

Sources that feed one layer must select the same fields and the same geometry. GDAL writes
appended features into the fields the layer was made with, and gives no message. A source
that selects fewer fields than the source before it thus deletes a field from each feature
in the layer.
