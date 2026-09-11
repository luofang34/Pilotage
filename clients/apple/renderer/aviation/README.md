# Aviation map renderer

The build uses the renderer revision in `MAPLIBRE_GLOBE_REVISION`.
A pinned input is an input with a recorded cryptographic digest.
`build.json` records the patch, file, font, and dependency lock digests.
The build stops if a required input does not match its digest.

The patch adds the IFR Map Lab chart symbol and line pipelines.
These pipelines use paper dimensions, symbol paths, and collision regions.
The globe symbol pipeline remains available for ordinary map styles.
The native host limits vector storage by bytes.

The Apple map uses one renderer for all installed profiles.
Each profile has a declared display group in the style metadata.
A profile change selects that group and keeps the map camera.
Common layers and live display values remain available in each profile.
The native host projects live display positions with the same map camera.
The camera uses north-up orientation at world scale.

Each vector source selects its own available tiles.
Missing regional detail can use a coarser tile from the same source.
A successful empty tile does not use features from a coarser tile.
Tile masks and chart collision regions use the selected source geometry.
The GPU tests check overlapping masks and sources with different detail levels.
The cache retains requested ancestors for each vector source.
The polygon reader ignores rings with zero area.
A failed source tile uses an available ancestor.

The native host reads `pilotage:resources` from the style metadata.
This field contains `ResourceBinding` records from `pilotage-map-archives`.
Each record binds a `pilotage://` URI to one installed file.
Tile requests append `/z/x/y` to an archive URI.
Sprite requests use an exact file URI.
The host shares these bindings with its tile workers.
A missing local tile does not cause a network request.
The package store must retain each release while the map uses its files.

The build reads font inputs from the local IFR Map Lab repository.
Set `IFR_MAP_LAB_SOURCE` if this repository is not beside Pilotage.
Set `MAPLIBRE_GLOBE_SOURCE` to a local renderer repository that contains the
required revision.
Run `clients/apple/scripts/build-maplibre-globe.sh` to build the native library.
The build creates a separate source directory for each aviation configuration.

Run the preparation guard tests with:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 clients/apple/scripts/test-prepare-maplibre-aviation.py
```

The renderer tests include chart frame draws and globe image checks.
These tests require a graphics processing unit (GPU).
The `chart_snapshot` example reads a style with local resource bindings.
It writes two PNG frames in the current directory.
Use these images to inspect a data release before installation on a device.

Font input verification does not establish permission to distribute a font.
The data publication record must include the required distribution rights.
