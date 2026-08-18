#!/usr/bin/env bash
# Prove that the attribution guard rejects a source that credits nobody.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture="$(mktemp -d "${TMPDIR:-/tmp}/pilotage-attribution.XXXXXX")"
trap 'rm -rf "$fixture"' EXIT

app="$fixture/clients/apple/App"
resources="$fixture/clients/apple/Resources"
mkdir -p "$app" "$resources" "$fixture/scripts"
cp "$repo_root/clients/apple/App/MapModesView.swift" "$app/"
cp "$repo_root/clients/apple/App/PilotageApp.swift" "$app/"
cp "$repo_root/clients/apple/App/SituationContentView.swift" "$app/"
cp "$repo_root/clients/apple/App/SituationStyleResource.swift" "$app/"
cp "$repo_root/clients/apple/Resources/SituationStyle.json" "$resources/"
cp "$repo_root/scripts/check-situation-map-attribution.sh" "$fixture/scripts/"

gate="$fixture/scripts/check-situation-map-attribution.sh"
bash "$gate" "$fixture" >/dev/null

reject() {
    if bash "$gate" "$fixture" >/dev/null 2>&1; then
        echo "the attribution guard accepted $1" >&2
        exit 1
    fi
}

python3 - "$resources/SituationStyle.json" <<'PY'
import json
import sys
path = sys.argv[1]
style = json.load(open(path))
style["sources"]["pilotage-uncredited"] = {"type": "raster", "tiles": ["http://x/{z}/{x}/{y}"]}
json.dump(style, open(path, "w"))
PY
reject "a source that draws without a notice"
cp "$repo_root/clients/apple/Resources/SituationStyle.json" "$resources/"

python3 - "$resources/SituationStyle.json" <<'PY'
import json
import sys
path = sys.argv[1]
style = json.load(open(path))
style["sources"]["pilotage-coastline"]["attribution"] = "Made with a renamed provider."
style["sources"]["pilotage-terrain"]["attribution"] = "Heights from a renamed provider."
json.dump(style, open(path, "w"))
PY
reject "notices no provider name in the panel matches"
cp "$repo_root/clients/apple/Resources/SituationStyle.json" "$resources/"

sed -i.bak 's/Button(role: .close, action: close)/Button(action: close)/' "$app/MapModesView.swift"
reject "a panel closed by something other than the platform's close button"
cp "$repo_root/clients/apple/App/MapModesView.swift" "$app/"

sed -i.bak 's/Image(systemName: "xmark")/Text(verbatim: "")/' "$app/MapModesView.swift"
reject "a close button that renders the word the role carries"
cp "$repo_root/clients/apple/App/MapModesView.swift" "$app/"

sed -i.bak '/buttonStyle(.glass)/d' "$app/MapModesView.swift"
reject "a close button with no disc under its cross"
cp "$repo_root/clients/apple/App/MapModesView.swift" "$app/"

sed -i.bak '/static func attributions/d' "$app/SituationStyleResource.swift"
reject "notices read from a loaded map rather than from the style document"
cp "$repo_root/clients/apple/App/SituationStyleResource.swift" "$app/"

sed -i.bak 's/drawsSurface: false/drawsSurface: true/' "$app/SituationContentView.swift"
reject "a panel drawing a second surface inside the sheet"
cp "$repo_root/clients/apple/App/SituationContentView.swift" "$app/"

sed -i.bak '/presentationDetents/d' "$app/SituationContentView.swift"
reject "a panel that lost its narrow presentation"
cp "$repo_root/clients/apple/App/SituationContentView.swift" "$app/"

sed -i.bak 's/min(modesHeight, windowHeight \* 0.92)/modesHeight/' "$app/SituationContentView.swift"
reject "a sheet that may ask to be taller than its window"
cp "$repo_root/clients/apple/App/SituationContentView.swift" "$app/"

sed -i.bak 's/horizontalSizeClass == .regular/true/' "$app/SituationContentView.swift"
reject "a panel that never asks whether there is room beside the map"
cp "$repo_root/clients/apple/App/SituationContentView.swift" "$app/"

bash "$gate" "$fixture" >/dev/null
echo "attribution and presentation guards reject each loss"
