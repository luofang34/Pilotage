# maplibre-rs iOS offline source spike

## Decision

Pilotage will not use `maplibre-rs` for the MVP iOS client. The MVP will use
MapLibre Native.

The local source adapter is a candidate `maplibre-rs` change. It changes general
renderer input and has no Pilotage display policy. Pilotage keeps the candidate
patch on a fork. An upstream submission requires explicit approval.

The adapter can read and draw the Pilotage vector archive. The supported iOS
27 application path cannot start the renderer. Thus, the physical iPad draw
test and the frame-rate comparison have no valid result.

## Scope

The spike used
[maplibre-rs commit 96b50a0](https://github.com/maplibre/maplibre-rs/commit/96b50a09d4925f46bd68e184e754d22ac3e356a2)
as its base. The adapter implementation is
[commit a5c055e6](https://github.com/luofang34/maplibre-rs/commit/a5c055e6f836f16a5499baf57523fb78d0ebef1d).

A local source adapter is a source client that reads an installed archive. It
does not use an HTTP client.

## Archive format

The adapter uses MBTiles 1.3. It opens the SQLite file in read-only mode. It
converts XYZ tile rows to TMS tile rows. It decompresses gzip tile data before
it gives the data to the renderer.

Pilotage uses MBTiles for these reasons:

- MapLibre Native has a tested iOS MBTiles path.
- The Navdata builder produces one deterministic MBTiles file.
- SQLite gives an indexed read for one tile.
- The metadata table carries the Navdata cycle identity.
- One archive is easier to install than a tile directory.
- The spike did not validate PMTiles on iOS in either renderer.

See [Navdata baseline tile bundle](navdata-baseline-tile-bundle.md) for the
archive schema.

## Adapter design

The adapter implements the existing `HttpClient` interface. The name of this
interface does not limit the implementation to HTTP. The Apple runner installs
the MBTiles client in the main kernel and in each worker kernel. The local path
does not construct `ReqwestHttpClient`.

The Apple framework has these host interfaces:

- A C function accepts an archive path and style JSON.
- A Swift function accepts an archive file URL and style JSON.
- The Swift package example resolves both resources from the application
  bundle.

The static Apple library includes SQLite. The application does not need a
separate SQLite link step.

## Validation

The following checks passed on 2026-08-09.

| Check | Result |
|---|---|
| Pinned format check | Pass. |
| `maplibre` library tests | 39 passed and 2 ignored. |
| MBTiles adapter tests | 8 passed. |
| Apple boundary tests | 4 passed. |
| Apple clippy check for `aarch64-apple-ios` | Pass. |
| `maplibre-winit` clippy check | Pass. |
| Rust check for `aarch64-apple-ios` | Pass. |
| Unsigned Xcode iOS application build | Pass with the iOS 27 SDK. |

The MBTiles tests use the host name `must-not-resolve.invalid`. The test gets a
tile from SQLite and verifies the bytes. It also verifies that the archive bytes
do not change. Other tests cover gzip data, data without compression, invalid
coordinates, required tables, and a normalized `tiles` view.

## Archive draw evidence

The macOS Apple runner used the reviewed Navdata archive. The archive SHA-256
was `10a7f0392a5f1d0c653f03725d0e5b4c470a1940e1de9bf8b872de2b0e1788cf`.
Its cycle identity was `faa-nasr:2026-06-11`.

The runner requested tile `6/18/24` from the archive. It decoded the vector
tile. It made render commands for the `airways` layer. The trace also showed
render commands for adjacent archive tiles. The local path did not install a
network source client.

This result proves the archive-to-render-command path on Apple hardware. It is
not an iPad performance result.

## Physical iPad result

The physical test used an iPad Pro 11-inch with an M4 processor. Its model was
`iPad16,4`. It used iOS 27.0 build `24A5390f`.

The signed application installed on the device. A normal SDK 27 build stopped
before renderer startup. The runtime reported that the application had no
scene lifecycle. `winit` 0.30.13 starts iOS with `UIApplicationMain` and the
application delegate lifecycle. It does not provide the required scene
lifecycle for this application path.

A diagnostic build with a changed SDK identity reached a background frame.
This build was not a supported application build. It is not acceptance
evidence.

The final USB CDC check found no device. A local-network device connection
cannot prove that the radio interfaces are disabled. Thus, this spike does not
claim a map draw with disabled radio interfaces.

## Performance result

No supported `maplibre-rs` frame reached the physical iPad display. Therefore,
there is no valid cold-start time or pan-and-zoom frame rate. A numerical
comparison with MapLibre Native would not compare the same scene.

This is a No-Go result. A renderer that cannot produce its first supported
frame has no usable frame-rate result.

## Question 5: offline vector data

Yes for the source adapter. It reads the installed vector archive without an
HTTP client. No for the supported Pilotage iOS application path. The iOS 27
lifecycle stop occurs before the adapter can request a tile.

## Carried renderer questions

### Question 1: regional raster precipitation

No. The Apple runner does not register its raster plug-in. The spike did not
find a supported path that draws the regional precipitation grid.

### Question 6: live feature state

No. The active renderer API has no feature-state operation that can restyle an
existing tile feature from live data. A legacy internal type does not provide a
host API.

### Question 7: one cycle identity for two sources

No. The renderer does not compare Navdata cycle identities across sources.
Pilotage must compare the archive metadata and the live source identity before
it combines their presentation data. This rule belongs above the renderer.

### Question 8: offline raster-dem data

Yes for MapLibre Native 6.28.0. The
[physical iPad result](https://github.com/luofang34/Pilotage/issues/351#issuecomment-5233829710)
used a bundled MBTiles archive and made no HTTP or HTTPS request.

No for `maplibre-rs`. Its style source types are vector, raster, and GeoJSON.
It has no `raster-dem` source type. The Apple runner has no hillshade plug-in.

## Re-evaluation conditions

Re-evaluate `maplibre-rs` for iOS only after all these conditions are true:

- The normal application build uses the required iOS scene lifecycle.
- A physical USB-connected iPad draws from the archive with its radio
  interfaces disabled.
- The same scene has a cold-start and frame-rate comparison with MapLibre
  Native.
- The required raster and `raster-dem` paths have physical-device evidence.
- The host has a supported live feature-state interface, if the product needs
  that interface.
