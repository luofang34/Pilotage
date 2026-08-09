#!/usr/bin/env bash
# Prove that the situation layer control guard rejects policy drift.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture="$(mktemp -d "${TMPDIR:-/tmp}/pilotage-layer-controls.XXXXXX")"
trap 'rm -rf "$fixture"' EXIT

portable="$fixture/crates/pilotage-presentation/src"
ffi="$fixture/clients/apple-situation/rust/pilotage-situation-ffi/src"
app="$fixture/clients/apple-situation/App"
binding="$fixture/clients/apple-situation/Packages/PilotageMapLibreBinding/Sources/PilotageMapLibreBinding"
mkdir -p "$portable/tests" "$ffi" "$app" "$binding" "$fixture/scripts"

cp "$repo_root/crates/pilotage-presentation/src/layer.rs" "$portable/"
cp "$repo_root/crates/pilotage-presentation/src/model.rs" "$portable/"
cp "$repo_root/crates/pilotage-presentation/src/policy.rs" "$portable/"
cp "$repo_root/crates/pilotage-presentation/src/detail.rs" "$portable/"
cp "$repo_root/crates/pilotage-presentation/src/tests/traffic.rs" "$portable/tests/"
cp "$repo_root/clients/apple-situation/rust/pilotage-situation-ffi/src/session.rs" "$ffi/"
cp "$repo_root/clients/apple-situation/App/PilotageSituationApp.swift" "$app/"
cp "$repo_root/clients/apple-situation/Packages/PilotageMapLibreBinding/Sources/PilotageMapLibreBinding/"*.swift "$binding/"
cp "$repo_root/scripts/check-situation-layer-controls.sh" "$fixture/scripts/"

bash "$fixture/scripts/check-situation-layer-controls.sh" "$fixture" >/dev/null

sed -i.bak 's/[.]age_micros(now_micros)/.lost_age(now_micros)/' "$portable/detail.rs"
if bash "$fixture/scripts/check-situation-layer-controls.sh" "$fixture" >/dev/null 2>&1; then
    echo "the layer control guard accepted traffic detail without field age" >&2
    exit 1
fi
sed -i.bak 's/[.]lost_age(now_micros)/.age_micros(now_micros)/' "$portable/detail.rs"

printf '\nlet forbiddenLayerPolicy = "weather-reports"\n' >> "$binding/SituationOverlay.swift"
if bash "$fixture/scripts/check-situation-layer-controls.sh" "$fixture" >/dev/null 2>&1; then
    echo "the layer control guard accepted layer policy in the map binding" >&2
    exit 1
fi
sed -i.bak '/forbiddenLayerPolicy/d' "$binding/SituationOverlay.swift"

sed -i.bak 's/visibleFeatures(/hiddenFeatures(/' "$binding/SituationMapView.swift"
if bash "$fixture/scripts/check-situation-layer-controls.sh" "$fixture" >/dev/null 2>&1; then
    echo "the layer control guard accepted a missing tap query" >&2
    exit 1
fi

echo "situation layer controls self-test: OK"
