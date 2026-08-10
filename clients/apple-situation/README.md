# Pilotage situation client

This client shows traffic and weather on iPadOS.

The Rust facade links the Surveillance and Airmass domain crates. It builds
three Apple library slices. The build script puts these slices in one
XCFramework. UniFFI generates the Swift binding from the same library.

The portable Rust adapter supplies all overlay values and styles. The Swift
binding maps these values to MapLibre Native 6.28.0. The binding does not read
domain snapshots.

The portable adapter consumes typed feature changes from
`surveillance-geojson` and `airmass-geojson`. Airmass supplies each flight
category. The Rust adapter maps each category to a display style.

The application uses one AeroLink discovery object. A maintenance task opens
and starts receiver connections. A separate task drains each live connection.
The drain has a limit of 32 transfers for each 20 millisecond cycle.

The application splits the AeroLink output into lines. The Rust radio adapter
converts each line into typed Surveillance and Airmass records. The application
routes these records to the presentation session. Swift does not decode a radio
payload.

The application stops reception when its scene is not active. It removes the
retained traffic and weather display values at the same time.

Run this command:

```sh
sh clients/apple-situation/scripts/ci-ios.sh
```

The command checks the Rust facade. It builds the XCFramework. It tests the
GeoJSON edge. It then builds the MapLibre binding for the iOS Simulator.

The default build uses the reviewed MapLibre Native 6.28.0 distribution. The
optional terrain build uses a pinned, unreleased MapLibre Native source. The
6.28.0 renderer has no 3D terrain support. The optional build keeps the
unreleased source out of the default product build.

Prepare a clean `WifiDB/maplibre-native` worktree at the commit in
`MAPLIBRE_TERRAIN_REVISION`. Initialize all its submodules. Install Bazel.
Then run this command:

```sh
PILOTAGE_MAPLIBRE_TERRAIN=1 \
MAPLIBRE_TERRAIN_SOURCE=/absolute/path/to/maplibre-native \
sh clients/apple-situation/scripts/ci-ios.sh
```

The script uses the MapLibre Native Metal XCFramework target. It puts the
binary in an ignored local Swift package. The same flag adds the style terrain
root when the script builds the application. Run all terrain demo builds with
the flag set. Do not use this build as the released distribution.

## Generate the iPadOS project

Clone AeroLink next to this repository. Check out the commit in
`AERO_LINK_REVISION`. Then run this command:

```sh
sh clients/apple-situation/scripts/generate-project.sh
```

The script copies the pinned AeroLink source to an ignored build directory.
It generates one AeroLink project for this app. The app embeds the generated
client framework and driver extension.

Clone Airmass and Surveillance next to this repository. The Apple check stages
the commits in `AIRMASS_REVISION` and `SURVEILLANCE_REVISION`. You can set
`AIRMASS_SOURCE` and `SURVEILLANCE_SOURCE` when the worktrees are in a different
directory.

The staging script applies the Pilotage DriverKit development entitlements to
the copied driver. These entitlements use one USB vendor wildcard. The copied
driver property list keeps the exact receiver match table.

The default App ID pair is:

- Host: `org.luofang.pilotage.situation`
- Driver: `org.luofang.pilotage.situation.aerolink-driver`

Set `AERO_LINK_HOST_BUNDLE_IDENTIFIER` and
`AERO_LINK_DRIVER_BUNDLE_IDENTIFIER` to use a different pair. The driver App
ID must begin with the host App ID.

Create both explicit App IDs in your Apple developer account. Add the System
Extension capability and the DriverKit communication capability to the host
App ID. Add the DriverKit capability to the driver App ID. Use the self-service
USB transport development entitlement with its vendor wildcard. Use automatic
signing for a development build. This development setup does not need an Apple
entitlement request. Distribution approval is separate and is not part of this
client.

The AeroLink harness and this app can both match one radio. The iPadOS Settings
app shows the installed drivers. Enable the Pilotage driver and disable the
harness driver before you start this app. The continuous integration check
proves that the two hosts use different App ID pairs and the same radio match
table. A physical iPad test must also verify the Settings selection.

Generated bindings and binary artifacts are build outputs. Do not commit these
outputs.
