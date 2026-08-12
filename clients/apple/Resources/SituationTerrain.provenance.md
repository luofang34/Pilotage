# Situation terrain archive

This archive holds published elevation tiles. It is not surveyed terrain for flight. Do
not use it for terrain separation, for an obstacle clearance decision, or for any other
operational purpose. It gives the map a shaded surface and a height to drape features on.

## Source

Tiles come from AWS Terrain Tiles, in Terrarium encoding, 256 pixels square. That dataset
combines several public sources, and every published map must carry the attribution:

> Elevation from AWS Terrain Tiles. Sources include SRTM, ASTER GDEM, NRCan CDEM, and USGS 3DEP.

The style carries the same text on the `pilotage-terrain` source, so the renderer shows it.

## What the archive contains

`SituationTerrain.plan.json` selects the tiles and is committed. The plan has two bands:

- a world band at low zoom, so a map zoomed out shows the shape of the globe rather than
  empty ocean;
- a regional band at the zoom a pilot reads, over the area the aircraft flies.

A `raster-dem` source overzooms past its highest zoom, so the regional band stops below
the closest zoom the map allows.

## Building it

The archive is a build artifact. It is large, its contents come from a tile service rather
than from this repository, and it is not committed. Build it from the repository root:

```text
sh clients/apple/scripts/build-situation-terrain.sh
```

The script caches each tile under `clients/apple/.build/terrain-tiles`, so a
second run fetches only what is missing. It writes `SituationTerrain.manifest.json`, which
records the plan digest, the archive digest, the tile counts, and the bands. That manifest
is committed and is what the terrain check verifies.

Pass `--force` to rebuild an archive that already matches its manifest.

## Replacing the source

A data producer that publishes its own DEM replaces the plan with its own tile service and
rebuilds. Nothing else in the client changes: the style names one `raster-dem` source and
reads whatever the archive holds.
