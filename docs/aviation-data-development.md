# Aviation data development

`pilotage-data-packages` owns package verification, installation, and selection.
`pilotage-data-delivery` owns signed catalog updates and HTTPS downloads.
`pilotage-map-archives` reads PMTiles and MBTiles files from persistent storage.
The navigation data crate decodes `.acnav` files.
The package crates do not define navigation records.

The Apple app stores installed packages in Application Support.
It excludes this directory from device backup.
The renderer uses the cache directory for data that it can obtain again.
The app does not put installed packages in the cache directory.

The Data screen is under More.
It shows the edition, coverage, and validity of each installed release.
All validity times use UTC.
An expired release stays installed.
A retained selection prevents removal.
A download does not change a map or flight selection.

## Prepare local examples

Import a producer release into a local package store:

```sh
cargo run -p pilotage-data-packages --example install_local -- \
  /path/to/release.json /path/to/source /path/to/store
```

Copy installed development releases into the Apple bundle:

```sh
python3 clients/apple/scripts/prepare-aviation-data-examples.py \
  /path/to/store navdata-existing-2026-06-11 terrain-existing \
  basemap-natural-earth-r2 ifr-low-2609-r2 ifr-high-2609-r2 procedures-2609-kttn-r1
```

List dependency releases before the releases that use them.
The script checks each file size and SHA-256 digest.
The app verifies each package before it records the installation.
The app imports each bundled example once.
Removal does not cause another import at the next start.

The coastline builder extends each detail region to complete tile boundaries.
It removes adjacent tiles that contain only polygon buffer fragments.
The source manifest records the resulting tile coverage.
Use a new release ID and revision when archive bytes change.

## Use installed data

The map and the presentation session use the selected terrain package.
Use `Use this edition` in Data to select a terrain or base map release.
The presentation session uses terrain to place traffic and weather above the surface.
Select a navigation edition in Data to load its weather station positions.
The app restores this selection at the next start.
Use `Inspect this edition` to select an expired development example.
The Data screen continues to show its expired state.

Terrain, IFR Low, and IFR High use one globe renderer.
A content change retains the camera and loaded resources.
The overview keeps north at the top.
Terrain mode uses the installed elevation data to draw the ground surface in 3D.
Drag with two fingers to tilt the map.
Select `3D` to tilt the local view.
Select `2D` to look straight down.
The globe overview hides the tilt control.
Select the compass to turn back to north.
The source package controls the available ground detail.

Map controls use native SwiftUI Liquid Glass buttons and system label colors.
Each round control uses the shared `MapControlButton` view.
Its rendered size is 48 by 48 points.
The compass dial fills the control.
The map and location controls share a native glass pill.
The pill measures 48 by 104 points when both controls are available.
The iOS tests measure the control labels, compass dial, and pill.
The round control tests use light and dark appearance.
Glass identities use the system transition for each control.
The mode panel uses the regular Liquid Glass material.
The app does not add a tint to these controls.
The operating system controls the glass appearance and accessibility effects.
Use the system Reduce Motion setting to remove camera and control movement.
See [Apple materials guidance](https://developer.apple.com/design/human-interface-guidelines/materials).

The procedure viewer reads PDF files from the installed package.
It shows the package edition and validity state with each chart.
The PDF index must name artifacts in that release.
The viewer rejects a different edition, repeated chart IDs, and paths outside the release.
The package contains reference charts, not executable procedure legs.
The navigation snapshot still requires model and parser work for full CIFP procedure semantics.

Prepare a local procedure example from FAA source files:

```sh
python3 clients/apple/scripts/prepare-procedure-release.py \
  /path/to/d-tpp_Metafile.xml /path/to/pdfs /path/to/output \
  --id procedures-2609-kttn-r1 --airport KTTN --pdf 00982IL6.PDF
```

Use PDF files from the cycle in the FAA metadata.
The script retains the source metadata and exact PDF bytes.
It derives the UTC interval from the metadata.
It marks the example as partial coverage on the development channel.
Import this release with `install_local` before you add it to the app bundle.

The replay session uses the installed terrain and navigation data selected when it opens.
It retains its own navigation positions while it runs.
Historical package identities are not yet stored with reception recordings.
The package store supplies pinned selections for that integration.

## Configure updates

The host supplies a `Publisher` record.
It contains a publisher ID, HTTPS catalog URL, public signing keys, and channel.
The Apple host reads this record from `AviationPublisher.json` in its bundle.
The signing key does not come from the downloaded catalog.
The development build can run without a publisher configuration.

The publisher signs the exact catalog payload with Ed25519.
Each catalog has a sequence number and an expiry time.
The store retains the largest accepted sequence across app starts.
The store rejects a reused release ID with different content.
The store retains this identity record when a release leaves the catalog.

Downloads use immutable content digests.
An interrupted download retains its completed chunks.
The next request uses the stored byte offset.
The installer verifies the complete digest before it installs the release.
An expired catalog cannot authorize a new download.
Installed data remains available without a catalog request.

## Check the implementation

```sh
cargo test -p pilotage-data-packages -p pilotage-data-delivery -p pilotage-map-archives
sh clients/apple/scripts/test-aviation-data.sh
python3 clients/apple/scripts/test-prepare-procedure-release.py
```

The download tests use temporary loopback ports.
They check resumed, restarted, truncated, and invalid responses.
The Swift tests check UTC validity boundaries and retained expired selections.

## Base map selection

Use the Natural Earth package for the permanent world overview.
Keep this package installed when the user removes a detail region.
The terrain archive supplies separate elevation data.
It includes world overview tiles and more detailed regional tiles.

Protomaps is the preferred candidate for a more detailed base map.
Its OpenStreetMap data includes roads, water, buildings, and place names.
It supports regional PMTiles files and local storage.
See [Protomaps downloads](https://docs.protomaps.com/basemaps/downloads).
This candidate is not part of the installed examples.

Publish each base map edition through the existing package catalog.
Record its source date, data license, attribution, bounds, and zoom range.
Include its style, symbols, and fonts in the release dependencies.
Validate the complete tile coverage before publication.
Install each region before the map selects it.
Keep the current edition until the new edition passes all checks.

The renderer must retain coarse coverage while detail tiles load.
A partial set of detail tiles must not remove the remaining coarse coverage.
A change to the base map supplier must pass the globe and terrain image tests.
