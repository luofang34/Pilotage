#!/usr/bin/env bash
# Verify the reviewed dependencies and the private driver identity boundary.
set -euo pipefail

root="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
client="$root/clients/apple"
driver_entitlements="$client/Configuration/AeroLinkDriverDevelopment.entitlements"
maplibre_manifest="$client/Packages/PilotageMapLibreBinding/Package.swift"
core_manifest="$client/Packages/PilotageCore/Package.swift"
terrain_manifest="$client/Packages/PilotageMapLibreTerrain/Package.swift"
terrain_revision_file="$client/MAPLIBRE_TERRAIN_REVISION"
terrain_build="$client/scripts/build-maplibre-terrain.sh"
geojson_edge="$client/Packages/PilotageGeoJSONEdge/Sources/PilotageGeoJSONEdge/FeatureCollection.swift"
map_overlay="$client/Packages/PilotageMapLibreBinding/Sources/PilotageMapLibreBinding/SituationOverlay.swift"
ffi="$client/rust/pilotage-situation-ffi"
ffi_manifest="$ffi/Cargo.toml"
ffi_clippy_config="$ffi/clippy.toml"
ffi_lib="$ffi/src/lib.rs"
status=0

require_pattern() {
    local pattern=$1
    local file=$2
    local message=$3
    if ! grep -Eq "$pattern" "$file"; then
        echo "FORBIDDEN: $message" >&2
        status=1
    fi
}

require_fixed() {
    local value=$1
    local file=$2
    local message=$3
    if ! grep -Fq -- "$value" "$file"; then
        echo "FORBIDDEN: $message" >&2
        status=1
    fi
}

check_revision() {
    local file=$1
    local name=$2
    local revision
    revision=$(tr -d '[:space:]' < "$file")
    if [[ ! "$revision" =~ ^[0-9a-f]{40}$ ]]; then
        echo "FORBIDDEN: $name must contain one full commit identity" >&2
        status=1
    fi
    require_fixed "ref: $revision" "$root/.github/workflows/ci.yml" \
        "$name and the Apple job checkout must match"
}

check_ffi_lint_scope() {
    local lint_exemption unexpected_exemptions
    lint_exemption='#[allow(clippy::disallowed_types)]'
    if ! awk -v expected="$lint_exemption" '
        $0 == expected {
            if ((getline module_line) <= 0) {
                bad = 1
                next
            }
            if (module_line == "mod error;" || module_line == "mod records;" || module_line == "mod link;") {
                seen[module_line]++
            } else {
                bad = 1
            }
            next
        }
        /clippy::disallowed_types/ { bad = 1 }
        END {
            if (seen["mod error;"] != 1 || seen["mod records;"] != 1 || seen["mod link;"] != 1) {
                bad = 1
            }
            exit bad
        }
    ' "$ffi_lib"; then
        echo "FORBIDDEN: the UniFFI lint exemption must apply only to the error and record modules" >&2
        status=1
    fi
    unexpected_exemptions=$(grep -RInF 'clippy::disallowed_types' "$ffi/src" \
        | grep -vF "$ffi_lib:" || true)
    if [ -n "$unexpected_exemptions" ]; then
        echo "FORBIDDEN: no other FFI source module can allow disallowed types" >&2
        status=1
    fi
    if grep -RIn 'anyhow' "$ffi/src" >/dev/null; then
        echo "FORBIDDEN: the FFI source must not name anyhow" >&2
        status=1
    fi
    if grep -In 'anyhow' "$ffi_manifest" >/dev/null; then
        echo "FORBIDDEN: the standalone FFI crate must not depend on anyhow" >&2
        status=1
    fi
}

# The client lives at one path. A branch cut before the rename and merged after it
# brings the old path back with the whole tree under it, and both copies then compile:
# the stale one is never built and never noticed, and an edit made there is lost.
if [ -e "$root/clients/apple-situation" ]; then
    echo "FORBIDDEN: the Apple client has one home, and clients/apple-situation is not it" >&2
    status=1
fi

check_revision "$client/AERO_LINK_REVISION" AERO_LINK_REVISION
check_revision "$client/AIRMASS_REVISION" AIRMASS_REVISION
check_revision "$client/SURVEILLANCE_REVISION" SURVEILLANCE_REVISION

require_fixed 'unsafe_code = "forbid"' "$ffi_manifest" \
    "the standalone FFI crate must forbid unsafe code"
require_fixed 'disallowed_types = "deny"' "$ffi_manifest" \
    "the standalone FFI crate must deny disallowed types"
require_fixed 'path = "anyhow::Error"' "$ffi_clippy_config" \
    "the standalone FFI crate must disallow anyhow errors"
check_ffi_lint_scope
require_pattern 'exact: "6\.28\.0"' \
    "$maplibre_manifest" \
    "the MapLibre Native package must use the reviewed exact version"
require_pattern 'maplibre/maplibre-gl-native-distribution' \
    "$maplibre_manifest" \
    "the binding must use the official MapLibre Native distribution"
require_fixed '.linkedLibrary("sqlite3")' "$core_manifest" \
    "the situation core package must link the terrain archive database library"
require_pattern 'PILOTAGE_MAPLIBRE_TERRAIN' "$maplibre_manifest" \
    "the terrain renderer must require its explicit build flag"
require_pattern 'if terrainRendererEnabled' "$maplibre_manifest" \
    "the local terrain package must stay behind the explicit build flag"
require_fixed '.package(path: "../PilotageMapLibreTerrain")' "$maplibre_manifest" \
    "the terrain flag must select only the local reviewed package"
require_fixed 'path: "Artifacts/MapLibre.xcframework"' "$terrain_manifest" \
    "the terrain package must use the locally built XCFramework"
terrain_revision=$(tr -d '[:space:]' < "$terrain_revision_file")
if [[ ! "$terrain_revision" =~ ^[0-9a-f]{40}$ ]]; then
    echo "FORBIDDEN: MAPLIBRE_TERRAIN_REVISION must contain one full commit identity" >&2
    status=1
fi
require_fixed 'MAPLIBRE_TERRAIN_REVISION' "$terrain_build" \
    "the terrain build must read the reviewed commit identity"
require_pattern 'actual_revision=.*rev-parse HEAD' "$terrain_build" \
    "the terrain build must compare the source HEAD with the reviewed identity"
require_fixed "build_target='//platform/ios:MapLibre.dynamic'" "$terrain_build" \
    "the terrain build must use the MapLibre Native XCFramework target"
require_fixed '--//:renderer=metal' "$terrain_build" \
    "the terrain build must select the iOS Metal renderer"
require_fixed 'PILOTAGE_MAPLIBRE_TERRAIN must be 0 or 1' "$client/scripts/ci-ios.sh" \
    "the Apple check must reject an invalid terrain flag"
require_pattern 'build-maplibre-terrain[.]sh' "$client/scripts/ci-ios.sh" \
    "the flagged Apple check must build the terrain package"
require_pattern 'PILOTAGE_MAPLIBRE_SWIFT_CONDITIONS' "$client/project.yml" \
    "the application project must record the terrain build condition"
style_resource="$client/App/SituationStyleResource.swift"
terrain_style_count=$(grep -Fc 'style["terrain"]' "$style_resource" || true)
terrain_style_block=$(sed -n '/#if PILOTAGE_MAPLIBRE_TERRAIN/,/#endif/p' "$style_resource")
if [ "$terrain_style_count" -ne 1 ] || \
    ! grep -Fq 'style["terrain"]' <<<"$terrain_style_block"; then
    echo "FORBIDDEN: only the flagged style may enable the terrain root" >&2
    status=1
fi
require_fixed 'WifiDB/maplibre-native' "$client/README.md" \
    "the terrain source repository must be recorded"
require_pattern 'AeroLink/AeroLinkAppleClient' "$client/project.yml" \
    "the application must embed its AeroLink client copy"
require_pattern 'AeroLink/AeroLinkDriver' "$client/project.yml" \
    "the application must embed its AeroLink driver copy"
require_pattern 'Configuration/AeroLinkDriverDevelopment[.]entitlements' \
    "$client/scripts/prepare-aero-link.sh" \
    "the staged driver must use the Pilotage development entitlements"
require_pattern 'com[.]apple[.]developer[.]driverkit[.]transport[.]usb' \
    "$driver_entitlements" \
    "the DriverKit development entitlement must include USB transport"
require_pattern '<string>[*]</string>' "$driver_entitlements" \
    "the DriverKit development entitlement must use the vendor wildcard"
if grep -Eq 'idProduct|<integer>' "$driver_entitlements"; then
    echo "FORBIDDEN: the DriverKit development entitlement must match the self-service profile" >&2
    status=1
fi
if [ "$(grep -c '<key>idVendor</key>' "$driver_entitlements")" -ne 1 ]; then
    echo "FORBIDDEN: the DriverKit development entitlement must contain one vendor wildcard" >&2
    status=1
fi
require_pattern 'com\.apple\.developer\.driverkit\.communicates-with-drivers' \
    "$client/App/Pilotage.entitlements" \
    "the host must declare its DriverKit communication capability"
require_pattern 'com\.apple\.developer\.system-extension\.install' \
    "$client/App/Pilotage.entitlements" \
    "the host must declare its system extension capability"
require_pattern 'AeroLinkDriverBundleIdentifier' "$client/App/Info.plist" \
    "the host must identify its embedded driver"
require_pattern '"[$]AERO_LINK_HOST_BUNDLE_IDENTIFIER"[.][*]' \
    "$client/scripts/generate-project.sh" \
    "the project generator must require a nested driver App ID"
require_pattern 'brew install xcodegen' \
    "$root/.github/workflows/ci.yml" \
    "the Apple client job must install XcodeGen"
require_pattern 'ARCHS=arm64' \
    "$client/scripts/ci-ios.sh" \
    "the simulator build must use the available Rust architecture"
require_pattern 'prepare-radio-sources[.]sh' \
    "$client/scripts/ci-ios.sh" \
    "the Apple check must stage all exact radio sources"
require_pattern 'PilotageRadioSource' \
    "$client/scripts/ci-ios.sh" \
    "the Apple check must test the radio state package"
require_pattern 'AIRMASS_SOURCE.*external/Airmass' \
    "$root/.github/workflows/ci.yml" \
    "the Apple job must supply the pinned Airmass checkout"
require_pattern 'SURVEILLANCE_SOURCE.*external/Surveillance' \
    "$root/.github/workflows/ci.yml" \
    "the Apple job must supply the pinned Surveillance checkout"

runtime="$client/App/AeroLinkRadioRuntime.swift"
model="$client/App/SituationClientModel.swift"
radio_app="$client/App"
if [ "$(grep -Rhc 'ALDriverDiscovery()' "$radio_app"/*.swift | awk '{ total += $1 } END { print total + 0 }')" -ne 1 ]; then
    echo "FORBIDDEN: the host process must create one ALDriverDiscovery" >&2
    status=1
fi
require_pattern 'maximumTransfersPerCycle = 32' "$runtime" \
    "the drain must keep the reviewed 32-transfer limit"
require_pattern 'drainInterval = Duration[.]milliseconds[(]20[)]' "$runtime" \
    "the receiver drain must run every 20 milliseconds"
require_pattern 'maintenanceTask = Task[.]detached' "$model" \
    "blocking driver maintenance must use a detached task"
require_pattern 'drainTask = Task[.]detached' "$model" \
    "the receiver drain must use its own detached task"
require_pattern 'Task[.]detached.*utility' "$model" \
    "driver cleanup must not run on the main actor"
require_pattern 'await maintenance[.]value' "$model" \
    "suspension must wait for the maintenance worker to exit"
require_pattern 'await drain[.]value' "$model" \
    "suspension must wait for the drain worker to exit"
require_pattern 'if let cleanup = cleanupTask' "$model" \
    "activation must wait for an earlier suspension to finish"
require_pattern 'guard !Task[.]isCancelled, !isActive' "$model" \
    "a canceled activation must not restart radio reception"
require_pattern 'domain[.]acceptReceptionEvent' "$client/App/AeroLinkRadioState.swift" \
    "each opaque AeroLink line must enter the Rust domain router"
require_pattern 'session[.]acceptTrackRecord' "$client/App/AeroLinkRadioState.swift" \
    "typed traffic records must enter the presentation session"
require_pattern 'session[.]acceptWeatherRecord' "$client/App/AeroLinkRadioState.swift" \
    "typed weather records must enter the presentation session"
require_pattern 'session[.]loadTerrainArchiveBlocking' "$model" \
    "the application must open terrain before it presents vertical features"
require_pattern 'pilotage_terrain_query::TerrainArchive' "$ffi/src/session.rs" \
    "the FFI host must use the shared terrain archive reader"
require_fixed 'properties["uses_reported_altitude_fallback"]' "$geojson_edge" \
    "a terrain fallback must cross the display edge"
require_fixed 'NSPredicate(format: "below_terrain == NO")' "$map_overlay" \
    "the extrusion layer must reject negative heights"
require_fixed 'NSPredicate(format: "below_terrain == YES")' "$map_overlay" \
    "a flat fill must draw a negative height"
require_pattern 'split[(]whereSeparator: .[.]isNewline[)]' "$runtime" \
    "the host must split nonempty serialized event lines"
require_pattern 'guard batch[.]transferConsumed else' "$runtime" \
    "the drain must continue until AeroLink reports no consumed transfer"
require_pattern 'guard result[.]hasConsumedTransfer else' \
    "$client/App/AeroLinkRadioState.swift" \
    "an empty poll must not erase retained decoder counters"
require_pattern 'OSSystemExtensionsWorkspace[.]shared' \
    "$client/App/AeroLinkRadioDiscovery.swift" \
    "the disabled-driver state must use the system extension state"
require_pattern 'UIApplication[.]openSettingsURLString' \
    "$client/App/RadioStatusView.swift" \
    "the disabled-driver state must open this application settings page"
require_pattern 'source[.]availability == [.]driverDisabled' \
    "$client/App/RadioStatusView.swift" \
    "the settings action must appear only for a disabled driver"
require_pattern 'session[.]clearRadioRecords' "$client/App/AeroLinkRadioState.swift" \
    "suspension must clear retained radio display records"
require_pattern 'reconnectRequiredAfterScan[(]' "$client/App/AeroLinkRadioState.swift" \
    "a reconnect request raised during discovery must survive the scan result"

drain_body=$(sed -n '/private func drain[(]/,/^    }/p' "$runtime")
if grep -Eq '[.](status|start|stop)[(]' <<<"$drain_body"; then
    echo "FORBIDDEN: the drain worker contains a blocking lifecycle call" >&2
    status=1
fi
if grep -RInE 'JSONDecoder|JSONSerialization|"(payload|media_type)"' \
    "$radio_app"/AeroLinkRadio*.swift "$model"; then
    echo "FORBIDDEN: the Swift radio source parses a reception payload" >&2
    status=1
fi
if grep -RIn 'acceptReceptionEvents' "$radio_app" >/dev/null; then
    echo "FORBIDDEN: the Swift host must route typed domain records" >&2
    status=1
fi

require_pattern 'use surveillance_aero_link::replay' \
    "$ffi/src/reception/traffic.rs" \
    "traffic reception must enter the Surveillance AeroLink adapter"
require_pattern 'ingest_event[(]' "$ffi/src/reception/traffic.rs" \
    "traffic reception must call the Surveillance AeroLink adapter"
require_pattern 'airmass_aero_link::AeroLinkAdapter' \
    "$ffi/src/reception/weather.rs" \
    "weather reception must enter the Airmass AeroLink adapter"
require_pattern 'map_snapshot_transition' \
    "$ffi/src/session.rs" \
    "weather presentation must use typed Airmass feature changes"
require_pattern 'WeatherSnapshotRecord::new' \
    "$ffi/src/reception/weather.rs" \
    "Airmass must supply the canonical weather record"
require_pattern 'TrackRecord::new' \
    "$ffi/src/reception/traffic.rs" \
    "Surveillance must supply the canonical track record"
require_pattern 'serde_json::from_str::<ReceptionEvent>' \
    "$ffi/src/reception.rs" \
    "the Rust boundary must decode the portable AeroLink event"
require_pattern 'aero_link: aero_link::CURRENT_RECEPTION_SCHEMA_VERSION' \
    "$ffi/src/lib.rs" \
    "the facade must report its AeroLink schema"
require_pattern 'queueDepth.*queueCapacity' "$client/App/RadioStatusView.swift" \
    "the application must show queue depth and capacity"
require_pattern 'droppedTransfers.*droppedBytes' "$client/App/RadioStatusView.swift" \
    "the application must show dropped transfers and bytes"
require_pattern 'adsb1090GapSamples' "$client/App/RadioStatusView.swift" \
    "the application must show 1090 MHz decoder gaps"
require_pattern 'uat978GapCount' "$client/App/RadioStatusView.swift" \
    "the application must show 978 MHz decoder gaps"
require_pattern 'discardedUatBytes' "$client/App/RadioStatusView.swift" \
    "the application must show 978 MHz resynchronization bytes"
require_pattern 'drainLimitExhaustions' "$client/App/RadioStatusView.swift" \
    "the application must show bounded-drain exhaustion"

ownship_mark="$root/clients/apple/Packages/PilotageMapLibreBinding/Sources/PilotageMapLibreBinding/SituationOwnshipMark.swift"
ownship_motion="$root/clients/apple/Packages/PilotageCore/Sources/PilotageCore/OwnshipMotion.swift"
motion_corpus="$root/clients/situation-ownship-motion.corpus.json"

for required in "$ownship_mark" "$ownship_motion" "$motion_corpus"; do
    if [ ! -f "$required" ]; then
        echo "FORBIDDEN: ${required#"$root"/} is missing" >&2
        status=1
    fi
done

# The mark is aligned to the map, not the screen. The map can be turned and
# opens pitched, and a mark aligned to the screen points somewhere the aircraft
# is not for as long as either holds.
require_fixed 'symbol.textRotationAlignment = NSExpression(forConstantValue: "map")' \
    "$ownship_mark" \
    "the ownship mark must be aligned to the map rather than the viewport"
require_fixed 'symbol.textPitchAlignment = NSExpression(forConstantValue: "map")' \
    "$ownship_mark" \
    "the ownship mark's pitch must follow the map rather than the screen"

# A reader reads a direction off a point, so a mark whose heading nobody
# reported must not have one.
require_fixed 'heading == nil ? Self.pointlessGlyph : Self.pointedGlyph' \
    "$ownship_mark" \
    "a mark with no reported heading must have no point in it"

# The map draws in true north. A magnetic heading drawn as a true one is wrong
# by the local variation, which is tens of degrees in places.
require_fixed 'reference == .trueNorth' \
    "$ownship_motion" \
    "only a heading stated against true north may turn the mark"

# The two clients derive these directions in two languages and neither would
# notice the other drifting.
require_fixed 'trackFloorMetresPerSecond' \
    "$ownship_motion" \
    "the course floor must be the shared physical speed, not a round number in knots"

if [ "$status" -ne 0 ]; then
    echo "Apple client: FAILED" >&2
    exit 1
fi

echo "Apple client: OK"
