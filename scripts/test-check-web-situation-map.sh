#!/usr/bin/env bash
# Prove that the web situation map guard rejects each boundary loss.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture="$(mktemp -d "${TMPDIR:-/tmp}/pilotage-web-map.XXXXXX")"
trap 'rm -rf "$fixture"' EXIT

web="$fixture/clients/web"
mkdir -p "$web" "$fixture/scripts" "$fixture/.github/workflows"
for file in situation-map.js situation-style.js situation-camera.js \
    situation-ownship.js situation-ownship.test.mjs \
    situation-style.test.mjs situation-camera.test.mjs \
    situation-map.browser.test.mjs index.html main.js layout.js; do
    cp "$repo_root/clients/web/$file" "$web/"
done
for file in vendor-maplibre-web.sh build-web-situation-assets.sh \
    check-web-situation-map.sh; do
    cp "$repo_root/scripts/$file" "$fixture/scripts/"
done
cp "$repo_root/.github/workflows/ci.yml" "$fixture/.github/workflows/"
cp "$repo_root/.gitignore" "$fixture/"

# The guard asks git which files are committed, so the fixture is a repo.
git -C "$fixture" init -q
git -C "$fixture" add -A

gate="$fixture/scripts/check-web-situation-map.sh"
bash "$gate" "$fixture" >/dev/null

reject() {
    if bash "$gate" "$fixture" >/dev/null 2>&1; then
        echo "the web situation map guard accepted $1" >&2
        exit 1
    fi
}

restore() {
    for file in "$@"; do
        cp "$repo_root/$file" "$fixture/$file"
    done
    git -C "$fixture" add -A
}

cp "$repo_root/clients/apple/Resources/SituationStyle.json" "$web/SituationStyle.json"
git -C "$fixture" add -A
reject "a committed situation style fork under clients/web"
rm "$web/SituationStyle.json"
git -C "$fixture" rm -q --cached clients/web/SituationStyle.json

sed -i.bak '/^clients\/web\/vendor\/$/d' "$fixture/.gitignore"
reject "a vendor directory that is no longer a build artifact"
restore .gitignore

sed -i.bak '/^clients\/web\/situation-assets\.new\/$/d' "$fixture/.gitignore"
reject "an export staging directory that is no longer a build artifact"
restore .gitignore

printf 'import "./vendor/maplibre-gl/maplibre-gl.mjs";\n%s' \
    "$(cat "$web/situation-map.js")" >"$web/situation-map.js"
reject "a static renderer import on the boot path"
restore clients/web/situation-map.js

printf 'import {\n  Map as VendorMap,\n} from "./vendor/maplibre-gl/maplibre-gl.mjs";\n%s' \
    "$(cat "$web/situation-map.js")" >"$web/situation-map.js"
reject "a static renderer import split over lines"
restore clients/web/situation-map.js

printf '%s\nexport const VENDOR_HINT = "vendor/maplibre-gl";\n' \
    "$(cat "$web/situation-style.js")" >"$web/situation-style.js"
reject "a vendor reference in the style module"
restore clients/web/situation-style.js

sed -i.bak 's/await import(VENDOR_MODULE)/globalThis.maplibregl/' "$web/situation-map.js"
reject "a renderer that no longer loads through the vendored module"
restore clients/web/situation-map.js

sed -i.bak 's|const ASSETS_BASE|const REMOTE = "https://tiles.example.com/"; const ASSETS_BASE|' \
    "$web/situation-map.js"
reject "a run-time network URL in the map module"
restore clients/web/situation-map.js

sed -i.bak 's|const ASSETS_BASE|const REMOTE = "//tiles.example.com/"; const ASSETS_BASE|' \
    "$web/situation-map.js"
reject "a protocol-relative network URL in the map module"
restore clients/web/situation-map.js

sed -i.bak 's/maxZoom: deriveMaximumZoom(terrainManifest)/maxZoom: 15/' \
    "$web/situation-map.js"
reject "a closest zoom written by hand instead of derived from the manifest"
restore clients/web/situation-map.js

sed -i.bak 's/MAP_ASSETS_MISSING/MAP_ASSETS_ABSENT/g' "$web/situation-map.js"
reject "a map that lost its typed assets-missing state"
restore clients/web/situation-map.js

sed -i.bak '/NavigationControl/d' "$web/situation-map.js"
reject "a map a pointer cannot turn or tilt"
restore clients/web/situation-map.js

sed -i.bak 's/visualizePitch: true/visualizePitch: false/' "$web/situation-map.js"
reject "a compass a pointer cannot tilt with"
restore clients/web/situation-map.js

sed -i.bak 's/NORTH_UP_LABEL/"Facing north"/' "$web/situation-map.js"
reject "camera wording restated in the map module"
restore clients/web/situation-map.js

sed -i.bak 's/^MAPLIBRE_TARBALL_SHA256=.*/MAPLIBRE_TARBALL_SHA256=""/' \
    "$fixture/scripts/vendor-maplibre-web.sh"
reject "a renderer vendor without a pinned digest"
restore scripts/vendor-maplibre-web.sh

sed -i.bak '/situation-map.browser.test.mjs/d' "$fixture/.github/workflows/ci.yml"
reject "a CI workflow that no longer boots the map stage"
restore .github/workflows/ci.yml

sed -i.bak 's|run: node clients/web/situation-map.browser.test.mjs|# &|' \
    "$fixture/.github/workflows/ci.yml"
reject "a commented-out browser-test step"
restore .github/workflows/ci.yml

sed -i.bak 's|<script type="module" src="./main.js"></script>|<script type="module" src="./vendor/maplibre-gl/maplibre-gl.mjs"></script>\n<script type="module" src="./main.js"></script>|' \
    "$web/index.html"
reject "an index.html that references the vendor directory"
restore clients/web/index.html

bash "$gate" "$fixture" >/dev/null
# The mark's own clock is the one thing that removes it when the link goes
# silent. Nothing else in the client would notice.
python3 - "$fixture/clients/web/situation-ownship.js" <<'AGE_GONE'
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
before = source
source = source.replace("OWNSHIP_STALE_AFTER_MS", "OWNSHIP_HOLD_FOREVER_MS")
assert source != before, "the stale window is not written the way this case edits it"
open(path, "w", encoding="utf-8").write(source)
AGE_GONE
if bash "$fixture/scripts/check-web-situation-map.sh" "$fixture" >/dev/null 2>&1; then
    echo "the web situation guard accepted a mark that is never withdrawn" >&2
    exit 1
fi
cp "$repo_root/clients/web/situation-ownship.js" "$web/"

python3 - "$fixture/clients/web/main.js" <<'DRIVER_GONE'
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
before = source
source = source.replace("ageOwnship", "noSuchDriver")
assert source != before, "the driver is not written the way this case edits it"
open(path, "w", encoding="utf-8").write(source)
DRIVER_GONE
if bash "$fixture/scripts/check-web-situation-map.sh" "$fixture" >/dev/null 2>&1; then
    echo "the web situation guard accepted a mark clock nothing drives" >&2
    exit 1
fi
cp "$repo_root/clients/web/main.js" "$web/"

python3 - "$fixture/clients/web/situation-ownship.js" <<'REASON_GONE'
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
before = source
source = source.replace("OWNSHIP_NO_FIX", "OWNSHIP_SOMETHING")
assert source != before, "the reason is not written the way this case edits it"
open(path, "w", encoding="utf-8").write(source)
REASON_GONE
if bash "$fixture/scripts/check-web-situation-map.sh" "$fixture" >/dev/null 2>&1; then
    echo "the web situation guard accepted a mark with no typed reason" >&2
    exit 1
fi
cp "$repo_root/clients/web/situation-ownship.js" "$web/"
echo "web situation map guards reject each loss"
