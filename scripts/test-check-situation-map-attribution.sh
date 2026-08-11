#!/usr/bin/env bash
# Prove that the attribution guard rejects a source that credits nobody.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture="$(mktemp -d "${TMPDIR:-/tmp}/pilotage-attribution.XXXXXX")"
trap 'rm -rf "$fixture"' EXIT

app="$fixture/clients/apple-situation/App"
resources="$fixture/clients/apple-situation/Resources"
mkdir -p "$app" "$resources" "$fixture/scripts"
cp "$repo_root/clients/apple-situation/App/MapModesView.swift" "$app/"
cp "$repo_root/clients/apple-situation/App/SituationStyleResource.swift" "$app/"
cp "$repo_root/clients/apple-situation/Resources/SituationStyle.json" "$resources/"
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
cp "$repo_root/clients/apple-situation/Resources/SituationStyle.json" "$resources/"

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
cp "$repo_root/clients/apple-situation/Resources/SituationStyle.json" "$resources/"

sed -i.bak '/static func attributions/d' "$app/SituationStyleResource.swift"
reject "notices read from a loaded map rather than from the style document"
cp "$repo_root/clients/apple-situation/App/SituationStyleResource.swift" "$app/"

bash "$gate" "$fixture" >/dev/null
echo "attribution guard rejects an uncredited source"
