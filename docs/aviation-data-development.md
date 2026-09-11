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
  /path/to/store navdata-existing-2026-06-11 terrain-existing ifr-existing-2026-09-03
```

List dependency releases before the releases that use them.
The script checks each file size and SHA-256 digest.
The app verifies each package before it records the installation.
The app imports each bundled example once.
Removal does not cause another import at the next start.

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
```

The download tests use temporary loopback ports.
They check resumed, restarted, truncated, and invalid responses.
The Swift tests check UTC validity boundaries and retained expired selections.
