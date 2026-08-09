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
    "$client_fixture/rust/pilotage-situation-ffi/src/reception" \
    "$client_fixture/scripts" \
    "$fixture/scripts"
cp "$root/.github/workflows/ci.yml" "$fixture/.github/workflows/"
cp "$root/clients/apple-situation/AERO_LINK_REVISION" "$client_fixture/"
cp "$root/clients/apple-situation/AIRMASS_REVISION" "$client_fixture/"
cp "$root/clients/apple-situation/SURVEILLANCE_REVISION" "$client_fixture/"
cp "$root/clients/apple-situation/project.yml" "$client_fixture/"
cp "$root/clients/apple-situation/App/"AeroLinkRadio*.swift "$client_fixture/App/"
cp "$root/clients/apple-situation/App/RadioStatusView.swift" \
    "$root/clients/apple-situation/App/SituationClientModel.swift" \
    "$client_fixture/App/"
cp "$root/clients/apple-situation/App/Info.plist" "$client_fixture/App/"
cp "$root/clients/apple-situation/App/PilotageSituation.entitlements" "$client_fixture/App/"
cp "$root/clients/apple-situation/Configuration/AeroLinkDriverDevelopment.entitlements" \
    "$client_fixture/Configuration/"
cp "$root/clients/apple-situation/Packages/PilotageMapLibreBinding/Package.swift" \
    "$client_fixture/Packages/PilotageMapLibreBinding/"
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
cp "$root/scripts/check-apple-situation-client.sh" "$fixture/scripts/"

bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null

sed -i.bak 's/exact: "6\.28\.0"/exact: "6.27.0"/' \
    "$fixture/clients/apple-situation/Packages/PilotageMapLibreBinding/Package.swift"
if bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null 2>&1; then
    echo "the Apple situation client guard accepted dependency drift" >&2
    exit 1
fi

sed -i.bak 's/exact: "6\.27\.0"/exact: "6.28.0"/' \
    "$fixture/clients/apple-situation/Packages/PilotageMapLibreBinding/Package.swift"
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
printf '\nlet decoder = JSONDecoder()\n' \
    >> "$fixture/clients/apple-situation/App/AeroLinkRadioRuntime.swift"
if bash "$fixture/scripts/check-apple-situation-client.sh" "$fixture" >/dev/null 2>&1; then
    echo "the Apple situation client guard accepted payload parsing in Swift" >&2
    exit 1
fi

echo "Apple situation client guard self-test: OK"
