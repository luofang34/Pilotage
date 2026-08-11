#!/usr/bin/env bash
# Prove that the Apple situation client guard rejects dependency drift.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
client_fixture="$fixture/clients/apple-situation"
mkdir -p \
    "$fixture/.github/workflows" \
    "$client_fixture/App" \
    "$client_fixture/Configuration" \
    "$client_fixture/Packages/PilotageMapLibreBinding" \
    "$client_fixture/Packages/PilotageMapLibreBinding/Sources/PilotageMapLibreBinding" \
    "$client_fixture/Packages/PilotageSituationCore" \
    "$client_fixture/Packages/PilotageGeoJSONEdge/Sources/PilotageGeoJSONEdge" \
    "$client_fixture/Packages/PilotageMapLibreTerrain" \
    "$client_fixture/rust/pilotage-situation-ffi/src/reception" \
    "$client_fixture/scripts" \
    "$fixture/scripts"
cp "$root/.github/workflows/ci.yml" "$fixture/.github/workflows/"
cp "$root/clients/apple-situation/AERO_LINK_REVISION" "$client_fixture/"
cp "$root/clients/apple-situation/AIRMASS_REVISION" "$client_fixture/"
cp "$root/clients/apple-situation/SURVEILLANCE_REVISION" "$client_fixture/"
cp "$root/clients/apple-situation/MAPLIBRE_TERRAIN_REVISION" "$client_fixture/"
cp "$root/clients/apple-situation/README.md" "$client_fixture/"
cp "$root/clients/apple-situation/project.yml" "$client_fixture/"
cp "$root/clients/apple-situation/App/"AeroLinkRadio*.swift "$client_fixture/App/"
cp "$root/clients/apple-situation/App/RadioStatusView.swift" \
    "$root/clients/apple-situation/App/SituationClientModel.swift" \
    "$client_fixture/App/"
cp "$root/clients/apple-situation/App/Info.plist" "$client_fixture/App/"
cp "$root/clients/apple-situation/App/PilotageSituation.entitlements" "$client_fixture/App/"
cp "$root/clients/apple-situation/App/SituationStyleResource.swift" "$client_fixture/App/"
cp "$root/clients/apple-situation/Configuration/AeroLinkDriverDevelopment.entitlements" \
    "$client_fixture/Configuration/"
cp "$root/clients/apple-situation/Packages/PilotageMapLibreBinding/Package.swift" \
    "$client_fixture/Packages/PilotageMapLibreBinding/"
cp "$root/clients/apple-situation/Packages/PilotageSituationCore/Package.swift" \
    "$client_fixture/Packages/PilotageSituationCore/"
cp "$root/clients/apple-situation/Packages/PilotageMapLibreBinding/Sources/PilotageMapLibreBinding/SituationOverlay.swift" \
    "$client_fixture/Packages/PilotageMapLibreBinding/Sources/PilotageMapLibreBinding/"
cp "$root/clients/apple-situation/Packages/PilotageGeoJSONEdge/Sources/PilotageGeoJSONEdge/FeatureCollection.swift" \
    "$client_fixture/Packages/PilotageGeoJSONEdge/Sources/PilotageGeoJSONEdge/"
cp "$root/clients/apple-situation/Packages/PilotageMapLibreTerrain/Package.swift" \
    "$client_fixture/Packages/PilotageMapLibreTerrain/"
cp "$root/clients/apple-situation/scripts/build-maplibre-terrain.sh" "$client_fixture/scripts/"
cp "$root/clients/apple-situation/scripts/ci-ios.sh" "$client_fixture/scripts/"
cp "$root/clients/apple-situation/scripts/generate-project.sh" "$client_fixture/scripts/"
cp "$root/clients/apple-situation/scripts/prepare-aero-link.sh" "$client_fixture/scripts/"
cp "$root/clients/apple-situation/rust/pilotage-situation-ffi/src/lib.rs" \
    "$root/clients/apple-situation/rust/pilotage-situation-ffi/src/session.rs" \
    "$root/clients/apple-situation/rust/pilotage-situation-ffi/src/reception.rs" \
    "$client_fixture/rust/pilotage-situation-ffi/src/"
cp "$root/clients/apple-situation/rust/pilotage-situation-ffi/src/reception/traffic.rs" \
    "$root/clients/apple-situation/rust/pilotage-situation-ffi/src/reception/weather.rs" \
    "$client_fixture/rust/pilotage-situation-ffi/src/reception/"
cp "$root/clients/apple-situation/rust/pilotage-situation-ffi/Cargo.toml" \
    "$root/clients/apple-situation/rust/pilotage-situation-ffi/clippy.toml" \
    "$client_fixture/rust/pilotage-situation-ffi/"
cp "$root/scripts/check-apple-situation-client.sh" "$fixture/scripts/"

bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null

sed -i.bak 's/unsafe_code = "forbid"/unsafe_code = "deny"/' \
    "$client_fixture/rust/pilotage-situation-ffi/Cargo.toml"
if bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null 2>&1; then
    echo "the Apple situation client guard accepted an unsafe-code lint downgrade" >&2
    exit 1
fi
sed -i.bak 's/unsafe_code = "deny"/unsafe_code = "forbid"/' \
    "$client_fixture/rust/pilotage-situation-ffi/Cargo.toml"

sed -i.bak 's/disallowed_types = "deny"/disallowed_types = "warn"/' \
    "$client_fixture/rust/pilotage-situation-ffi/Cargo.toml"
if bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null 2>&1; then
    echo "the Apple situation client guard accepted a disallowed-type lint downgrade" >&2
    exit 1
fi
sed -i.bak 's/disallowed_types = "warn"/disallowed_types = "deny"/' \
    "$client_fixture/rust/pilotage-situation-ffi/Cargo.toml"

sed -i.bak 's/path = "anyhow::Error"/path = "removed::Error"/' \
    "$client_fixture/rust/pilotage-situation-ffi/clippy.toml"
if bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null 2>&1; then
    echo "the Apple situation client guard accepted the anyhow-error type" >&2
    exit 1
fi
sed -i.bak 's/path = "removed::Error"/path = "anyhow::Error"/' \
    "$client_fixture/rust/pilotage-situation-ffi/clippy.toml"

sed -i.bak 's/^#\[allow(clippy::disallowed_types/#![allow(clippy::disallowed_types/' \
    "$client_fixture/rust/pilotage-situation-ffi/src/lib.rs"
if bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null 2>&1; then
    echo "the Apple situation client guard accepted a crate-wide UniFFI lint exemption" >&2
    exit 1
fi
cp "$root/clients/apple-situation/rust/pilotage-situation-ffi/src/lib.rs" \
    "$client_fixture/rust/pilotage-situation-ffi/src/"

printf '%s\n' 'pub type DynamicError = uniffi::__anyhow::Error;' \
    >> "$client_fixture/rust/pilotage-situation-ffi/src/lib.rs"
if bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null 2>&1; then
    echo "the Apple situation client guard accepted anyhow in FFI source" >&2
    exit 1
fi
cp "$root/clients/apple-situation/rust/pilotage-situation-ffi/src/lib.rs" \
    "$client_fixture/rust/pilotage-situation-ffi/src/"

printf '%s\n' 'anyhow = "1"' \
    >> "$client_fixture/rust/pilotage-situation-ffi/Cargo.toml"
if bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null 2>&1; then
    echo "the Apple situation client guard accepted an anyhow dependency" >&2
    exit 1
fi
cp "$root/clients/apple-situation/rust/pilotage-situation-ffi/Cargo.toml" \
    "$client_fixture/rust/pilotage-situation-ffi/"

sed -i.bak 's/linkedLibrary("sqlite3")/linkedLibrary("removed-sqlite3")/' \
    "$fixture/clients/apple-situation/Packages/PilotageSituationCore/Package.swift"
if bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null 2>&1; then
    echo "the Apple situation client guard accepted a missing SQLite link" >&2
    exit 1
fi
sed -i.bak 's/linkedLibrary("removed-sqlite3")/linkedLibrary("sqlite3")/' \
    "$fixture/clients/apple-situation/Packages/PilotageSituationCore/Package.swift"

sed -i.bak 's/exact: "6\.28\.0"/exact: "6.27.0"/' \
    "$fixture/clients/apple-situation/Packages/PilotageMapLibreBinding/Package.swift"
if bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null 2>&1; then
    echo "the Apple situation client guard accepted dependency drift" >&2
    exit 1
fi

sed -i.bak 's/exact: "6\.27\.0"/exact: "6.28.0"/' \
    "$fixture/clients/apple-situation/Packages/PilotageMapLibreBinding/Package.swift"
sed -i.bak 's#../PilotageMapLibreTerrain#../UnreviewedTerrain#' \
    "$fixture/clients/apple-situation/Packages/PilotageMapLibreBinding/Package.swift"
if bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null 2>&1; then
    echo "the Apple situation client guard accepted an unreviewed terrain package" >&2
    exit 1
fi

sed -i.bak 's#../UnreviewedTerrain#../PilotageMapLibreTerrain#' \
    "$fixture/clients/apple-situation/Packages/PilotageMapLibreBinding/Package.swift"
sed -i.bak 's/--\/\/:renderer=metal/--\/\/:renderer=opengl/' \
    "$fixture/clients/apple-situation/scripts/build-maplibre-terrain.sh"
if bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null 2>&1; then
    echo "the Apple situation client guard accepted a non-Metal terrain build" >&2
    exit 1
fi

sed -i.bak 's/--\/\/:renderer=opengl/--\/\/:renderer=metal/' \
    "$fixture/clients/apple-situation/scripts/build-maplibre-terrain.sh"
sed -i.bak 's/#if PILOTAGE_MAPLIBRE_TERRAIN/#if true/' \
    "$fixture/clients/apple-situation/App/SituationStyleResource.swift"
if bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null 2>&1; then
    echo "the Apple situation client guard accepted an unflagged terrain style" >&2
    exit 1
fi

sed -i.bak 's/#if true/#if PILOTAGE_MAPLIBRE_TERRAIN/' \
    "$fixture/clients/apple-situation/App/SituationStyleResource.swift"
sed -i.bak 's/brew install xcodegen/brew install removed-xcodegen/' \
    "$fixture/.github/workflows/ci.yml"
if bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null 2>&1; then
    echo "the Apple situation client guard accepted a missing XcodeGen install" >&2
    exit 1
fi

sed -i.bak 's/brew install removed-xcodegen/brew install xcodegen/' \
    "$fixture/.github/workflows/ci.yml"
sed -i.bak 's/ARCHS=arm64/ARCHS=x86_64/' \
    "$fixture/clients/apple-situation/scripts/ci-ios.sh"
if bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null 2>&1; then
    echo "the Apple situation client guard accepted an unavailable simulator architecture" >&2
    exit 1
fi

sed -i.bak 's/ARCHS=x86_64/ARCHS=arm64/' \
    "$fixture/clients/apple-situation/scripts/ci-ios.sh"
sed -i.bak 's/<string>\*<\/string>/<integer>3034<\/integer>/' \
    "$fixture/clients/apple-situation/Configuration/AeroLinkDriverDevelopment.entitlements"
if bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null 2>&1; then
    echo "the Apple situation client guard accepted incompatible DriverKit entitlements" >&2
    exit 1
fi

sed -i.bak 's/<integer>3034<\/integer>/<string>*<\/string>/' \
    "$fixture/clients/apple-situation/Configuration/AeroLinkDriverDevelopment.entitlements"
sed -i.bak 's/maximumTransfersPerCycle = 32/maximumTransfersPerCycle = 31/' \
    "$fixture/clients/apple-situation/App/AeroLinkRadioRuntime.swift"
if bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null 2>&1; then
    echo "the Apple situation client guard accepted an unbounded drain change" >&2
    exit 1
fi

sed -i.bak 's/maximumTransfersPerCycle = 31/maximumTransfersPerCycle = 32/' \
    "$fixture/clients/apple-situation/App/AeroLinkRadioRuntime.swift"
sed -i.bak 's/WeatherSnapshotRecord::new/WeatherSnapshotRecord::invented/' \
    "$fixture/clients/apple-situation/rust/pilotage-situation-ffi/src/reception/weather.rs"
if bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null 2>&1; then
    echo "the Apple situation client guard accepted a noncanonical weather record" >&2
    exit 1
fi

sed -i.bak 's/WeatherSnapshotRecord::invented/WeatherSnapshotRecord::new/' \
    "$fixture/clients/apple-situation/rust/pilotage-situation-ffi/src/reception/weather.rs"
sed -i.bak '/var result = AeroLinkDrainResult()/a\
        let _ = try handle.value.status()' \
    "$fixture/clients/apple-situation/App/AeroLinkRadioRuntime.swift"
if bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null 2>&1; then
    echo "the Apple situation client guard accepted a blocking call on the drain worker" >&2
    exit 1
fi

sed -i.bak '/let _ = try handle.value.status()/d' \
    "$fixture/clients/apple-situation/App/AeroLinkRadioRuntime.swift"
sed -i.bak 's/await maintenance[.]value/await Task.yield()/' \
    "$fixture/clients/apple-situation/App/SituationClientModel.swift"
if bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null 2>&1; then
    echo "the Apple situation client guard accepted incomplete worker shutdown" >&2
    exit 1
fi

sed -i.bak 's/await Task[.]yield()/await maintenance.value/' \
    "$fixture/clients/apple-situation/App/SituationClientModel.swift"
sed -i.bak 's/reconnectRequiredAfterScan[(]/discardReconnectRequest(/' \
    "$fixture/clients/apple-situation/App/AeroLinkRadioState.swift"
if bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null 2>&1; then
    echo "the Apple situation client guard accepted a lost reconnect request" >&2
    exit 1
fi

sed -i.bak 's/discardReconnectRequest[(]/reconnectRequiredAfterScan(/' \
    "$fixture/clients/apple-situation/App/AeroLinkRadioState.swift"
sed -i.bak 's/if let cleanup = cleanupTask/if false/' \
    "$fixture/clients/apple-situation/App/SituationClientModel.swift"
if bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null 2>&1; then
    echo "the Apple situation client guard accepted overlapping lifecycle work" >&2
    exit 1
fi

sed -i.bak 's/if false/if let cleanup = cleanupTask/' \
    "$fixture/clients/apple-situation/App/SituationClientModel.swift"
sed -i.bak '/guard result[.]hasConsumedTransfer else { return }/d' \
    "$fixture/clients/apple-situation/App/AeroLinkRadioState.swift"
if bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null 2>&1; then
    echo "the Apple situation client guard accepted counter loss after an empty poll" >&2
    exit 1
fi

sed -i.bak '/let handle = connection[.]handle/i\
        guard result.hasConsumedTransfer else { return }' \
    "$fixture/clients/apple-situation/App/AeroLinkRadioState.swift"

sed -i.bak 's/loadTerrainArchiveBlocking/loadTerrainArchiveRemoved/' \
    "$fixture/clients/apple-situation/App/SituationClientModel.swift"
if bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null 2>&1; then
    echo "the Apple situation client guard accepted missing terrain archive loading" >&2
    exit 1
fi
sed -i.bak 's/loadTerrainArchiveRemoved/loadTerrainArchiveBlocking/' \
    "$fixture/clients/apple-situation/App/SituationClientModel.swift"

sed -i.bak 's/below_terrain == YES/below_terrain removed/' \
    "$fixture/clients/apple-situation/Packages/PilotageMapLibreBinding/Sources/PilotageMapLibreBinding/SituationOverlay.swift"
if bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null 2>&1; then
    echo "the Apple situation client guard accepted a missing negative-height fill" >&2
    exit 1
fi
sed -i.bak 's/below_terrain removed/below_terrain == YES/' \
    "$fixture/clients/apple-situation/Packages/PilotageMapLibreBinding/Sources/PilotageMapLibreBinding/SituationOverlay.swift"
printf '\nlet decoder = JSONDecoder()\n' \
    >> "$fixture/clients/apple-situation/App/AeroLinkRadioRuntime.swift"
if bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null 2>&1; then
    echo "the Apple situation client guard accepted payload parsing in Swift" >&2
    exit 1
fi

bash "$root/scripts/test-build-maplibre-terrain.sh"
echo "Apple situation client guard self-test: OK"
