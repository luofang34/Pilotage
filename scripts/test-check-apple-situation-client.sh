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
    "$client_fixture/Packages/PilotageMapLibreBinding" \
    "$client_fixture/scripts" \
    "$fixture/scripts"
cp "$root/.github/workflows/ci.yml" "$fixture/.github/workflows/"
cp "$root/clients/apple-situation/AERO_LINK_REVISION" "$client_fixture/"
cp "$root/clients/apple-situation/project.yml" "$client_fixture/"
cp "$root/clients/apple-situation/App/Info.plist" "$client_fixture/App/"
cp "$root/clients/apple-situation/App/PilotageSituation.entitlements" "$client_fixture/App/"
cp "$root/clients/apple-situation/Packages/PilotageMapLibreBinding/Package.swift" \
    "$client_fixture/Packages/PilotageMapLibreBinding/"
cp "$root/clients/apple-situation/scripts/ci-ios.sh" "$client_fixture/scripts/"
cp "$root/clients/apple-situation/scripts/generate-project.sh" "$client_fixture/scripts/"
cp "$root/scripts/check-apple-situation-client.sh" "$fixture/scripts/"

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

echo "Apple situation client guard self-test: OK"
