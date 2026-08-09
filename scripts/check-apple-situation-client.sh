#!/usr/bin/env bash
# Verify the fixed dependencies and the private driver identity boundary.
set -euo pipefail

root="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
client="$root/clients/apple-situation"
driver_entitlements="$client/Configuration/AeroLinkDriverDevelopment.entitlements"
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
    if ! grep -Fq "$value" "$file"; then
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

check_revision "$client/AERO_LINK_REVISION" AERO_LINK_REVISION
check_revision "$client/AIRMASS_REVISION" AIRMASS_REVISION
check_revision "$client/SURVEILLANCE_REVISION" SURVEILLANCE_REVISION

require_pattern 'exact: "6\.28\.0"' \
    "$client/Packages/PilotageMapLibreBinding/Package.swift" \
    "the MapLibre Native package must use the reviewed exact version"
require_pattern 'maplibre/maplibre-gl-native-distribution' \
    "$client/Packages/PilotageMapLibreBinding/Package.swift" \
    "the binding must use the official MapLibre Native distribution"
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
    "$client/App/PilotageSituation.entitlements" \
    "the host must declare its DriverKit communication capability"
require_pattern 'com\.apple\.developer\.system-extension\.install' \
    "$client/App/PilotageSituation.entitlements" \
    "the host must declare its system extension capability"
require_pattern 'AeroLinkDriverBundleIdentifier' "$client/App/Info.plist" \
    "the host must identify its embedded driver"
require_pattern '"[$]AERO_LINK_HOST_BUNDLE_IDENTIFIER"[.][*]' \
    "$client/scripts/generate-project.sh" \
    "the project generator must require a nested driver App ID"
require_pattern 'brew install xcodegen' \
    "$root/.github/workflows/ci.yml" \
    "the Apple situation client job must install XcodeGen"
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
ffi="$client/rust/pilotage-situation-ffi"
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

if [ "$status" -ne 0 ]; then
    echo "Apple situation client: FAILED" >&2
    exit 1
fi

echo "Apple situation client: OK"
