# Situation terrain fixture

This archive is a synthetic simulator fixture. It is not surveyed terrain.
Do not use it for flight.

The example creates one `SourceDataset`. The dataset contains one regular DEM
grid. The grid covers the world. The build selects Web Mercator zoom 0. The
stated region is 85.0511287798066 degrees south to 85.0511287798066 degrees
north. The longitude range is 180 degrees west to 180 degrees east.

Run this command from the repository root:

```text
cargo run --quiet -p pilotage-terrain-build --example build_situation_fixture -- clients/apple-situation/Resources/SituationTerrain.mbtiles
```

The warm debug build took 0.28 seconds on 2026-08-09. The archive size was
16,384 bytes. The SHA-256 digest was
`4bfb229fab057719778a65ee4b68569e16839998137bd5ddb401c5c20d00eaee`.

The application uses this fixture to test the offline delivery path. A data
producer must replace the fixture with an archive from its DEM
`SourceDataset`.
