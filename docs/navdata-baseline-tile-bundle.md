# Navdata baseline tile bundle

## Purpose

The build converts one identified Navdata snapshot to one offline tile bundle.
Run the build one time for each Navdata cycle. Do not run it on the client.

The bundle is a renderer-edge encoding. It is not domain state.

## Format decision

The bundle uses MBTiles 1.3. Each tile is a gzip-compressed Mapbox Vector Tile
2.1 Protocol Buffer.

MapLibre Native reads MBTiles from an application bundle on iOS. This path does
not require a local HTTP server. The `maplibre-rs` local source can read the same
SQLite rows and give the same Mapbox Vector Tile bytes to the renderer.

The build does not use PMTiles. The validated iOS path uses MBTiles. No validated
PMTiles path exists. A tile directory would use more files and would not add a
client capability.

The format follows the [MBTiles 1.3 specification](https://github.com/mapbox/mbtiles-spec/blob/master/1.3/spec.md)
and the [Mapbox Vector Tile 2.1 specification](https://github.com/mapbox/vector-tile-spec/blob/master/2.1/README.md).

## Identity

The metadata table has these required Pilotage rows.

| Row | Meaning |
|---|---|
| `pilotage_schema` | Tile bundle schema version. |
| `pilotage_cycle` | Navdata authority and effective date. |
| `pilotage_snapshot_id` | Immutable snapshot identity. |
| `pilotage_snapshot_digest` | Digest of the canonical snapshot payload. |

Each feature has `subject_id` and `subject_cycle` properties. The
`subject_id` value is stable in one cycle. A live overlay can use the same value
to name the baseline subject. The composition must compare the cycle values
before it joins two sources.

## Layer split

Schema version 1 has these baseline layers.

| Layer | First zoom | Geometry |
|---|---:|---|
| `airspaces` | 0 | Published horizontal bounds. |
| `airways` | 2 | Resolved published segments. |
| `aerodromes` | 3 | Published points. |
| `navaids` | 5 | Published points. |
| `fixes` | 6 | Published points. |

An airway gap does not make a line segment. An unresolved airway point does not
make a line segment. The report counts an airway as omitted when it has no
resolved segment.

The schema version 1 input has runway attributes but has no WGS84 runway-end
positions. The bundle omits runways and reports the count. A later schema can
add a runway layer when its Navdata input carries the required positions.

The schema version 1 input has no procedure collection. The bundle has no
procedure layer.

Airspace bounds can enclose more area than the exact boundary. The feature
property `geometry_quality=snapshot_bound` identifies this limit. Do not use the
tile geometry for a regulatory decision.

Weather, traffic, and aeronautical updates stay in live presentation layers.
They do not enter the cycle bundle.

## Offline access

The application installs the MBTiles file as a bundle resource. The Swift
binding gives MapLibre Native an absolute `mbtiles://` URL. The style does not
contain a device path.

`OfflineTileReader` opens installed archive bytes in a read-only SQLite
database. It has no network dependency. It verifies the schema and the three
Navdata identity values before it returns a tile.

## Reproducibility

The builder orders metadata, tiles, layers, properties, and values. It clips
geometry in Web Mercator coordinates. It uses a fixed tile extent. It writes a
gzip header with a zero time value. It writes a fixed SQLite schema and page
size, and then it runs `VACUUM`.

The same identified snapshot and the same configuration produce identical
archive bytes. The continuous integration test builds the same fixture two
times and compares all bytes.

## Operator command

Run this command with an `.acnav` snapshot blob and an output path.

```text
cargo run --release -p pilotage-navdata-tiles \
  --example build_navdata_tiles -- INPUT.acnav OUTPUT.mbtiles
```

The example decodes the blob before it calls the builder. The builder receives
only an `IdentifiedNavdataSnapshotV1` value.

## Full-cycle measurement

The measurement used the FAA NASR cycle effective on 2026-06-11. It used the
format version 4 snapshot. The host was an Apple M3 Max with 16 CPU cores and
48 GB of memory. The host used macOS 27.0 build 26A5368g and Rust 1.95.0. The
build used the release profile on 2026-08-09.

| Fact | Value |
|---|---:|
| Input size | 4,489,535 bytes |
| Input payload SHA-256 | `8be6d025d4731fd6353055fa8913516d959869971457fded6a0b6a67c9ef421a` |
| Input points | 93,595 |
| Input airways | 1,519 |
| Input runways | 23,178 |
| Input airspaces | 0 |
| Drawable aerodromes | 22,024 |
| Drawable navaids | 1,553 |
| Drawable fixes | 70,018 |
| Drawable airways | 1,414 |
| Populated tiles | 7,354 |
| Tile feature copies | 385,745 |
| Output size | 17,448,960 bytes |
| First build time | 2.276 seconds |
| Second build time | 2.400 seconds |
| Output SHA-256 from both builds | `10a7f0392a5f1d0c653f03725d0e5b4c470a1940e1de9bf8b872de2b0e1788cf` |

The two output files had identical bytes.
