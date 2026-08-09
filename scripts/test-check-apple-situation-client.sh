#!/usr/bin/env bash
# Prove that the Apple situation client guard rejects dependency drift.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
mkdir -p "$fixture/.github/workflows" "$fixture/clients" "$fixture/scripts"
cp "$root/.github/workflows/ci.yml" "$fixture/.github/workflows/"
cp -R "$root/clients/apple-situation" "$fixture/clients/"
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

echo "Apple situation client guard self-test: OK"
