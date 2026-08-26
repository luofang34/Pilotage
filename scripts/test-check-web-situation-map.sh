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
    situation-motion.js situation-motion.test.mjs \
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

# A clock nothing turns is a rule nothing enforces. The mutation removes
# the call that turns it and leaves the name in place, which is exactly the
# shape a refactor would leave behind.
python3 - "$fixture/clients/web/situation-map.js" <<'DRIVER_GONE'
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
before = source
source = source.replace(
    "setInterval(ageOwnship, OWNSHIP_AGE_INTERVAL_MS)", "null /* ageOwnship */", 1
)
assert source.count("setInterval(ageOwnship, OWNSHIP_AGE_INTERVAL_MS)") == 1, (
    "one call is the case: the other is the restore path, covered separately"
)
assert source != before, "the driver is not written the way this case edits it"
open(path, "w", encoding="utf-8").write(source)
DRIVER_GONE
if bash "$fixture/scripts/check-web-situation-map.sh" "$fixture" >/dev/null 2>&1; then
    echo "the web situation guard accepted a mark clock nothing drives" >&2
    exit 1
fi
cp "$repo_root/clients/web/situation-map.js" "$web/"

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
# Two lanes can carry the position and a reader has to be told which one is
# under the mark: an oracle is exact by construction, an estimate is a
# solution with an accuracy of its own.
python3 - "$web/situation-ownship.js" <<PY
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
before = source
source = source.replace('simulation-truth', "some-lane")
assert source != before, "the lane name is not written the way this case edits it"
open(path, "w", encoding="utf-8").write(source)
PY
if bash "$fixture/scripts/check-web-situation-map.sh" "$fixture" >/dev/null 2>&1; then
    echo "the web situation guard accepted a mark that names no lane" >&2
    exit 1
fi
cp "$repo_root/clients/web/situation-ownship.js" "$web/"

python3 - "$web/situation-ownship.js" <<PY
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
before = source
source = source.replace('ownshipSource', "ownshipWhence")
assert source != before, "the source attribute is not written the way this case edits it"
open(path, "w", encoding="utf-8").write(source)
PY
if bash "$fixture/scripts/check-web-situation-map.sh" "$fixture" >/dev/null 2>&1; then
    echo "the web situation guard accepted a mark that hides which measurement is under it" >&2
    exit 1
fi
cp "$repo_root/clients/web/situation-ownship.js" "$web/"

# A page restored from the back/forward cache resumes its telemetry. A
# clock stopped without re-arming leaves the mark updating and never
# ageing for the rest of the page's life.
python3 - "$web/situation-map.js" <<PY
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
before = source
source = source.replace('pageshow', "unload")
assert source != before, "the restore path is not written the way this case edits it"
open(path, "w", encoding="utf-8").write(source)
PY
if bash "$fixture/scripts/check-web-situation-map.sh" "$fixture" >/dev/null 2>&1; then
    echo "the web situation guard accepted a mark clock that a restored page never re-arms" >&2
    exit 1
fi
cp "$repo_root/clients/web/situation-map.js" "$web/"
# A pointed mark that never turns asserts screen-up on a map the reader
# can rotate.
python3 - "$web/situation-ownship.js" <<PY
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
before = source
source = source.replace('marker.setRotation(headingDeg ?? 0)', "marker.setLngLat(marker.lngLat)", 1)
assert source != before, "the rotation is not written the way this case edits it"
open(path, "w", encoding="utf-8").write(source)
PY
if bash "$fixture/scripts/check-web-situation-map.sh" "$fixture" >/dev/null 2>&1; then
    echo "the web situation guard accepted a mark that never turns" >&2
    exit 1
fi
cp "$repo_root/clients/web/situation-ownship.js" "$web/"

# A shape with a point in it states a direction whether or not one was
# measured.
python3 - "$web/situation-ownship.js" <<PY
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
before = source
source = source.replace('map-ownship-unknown-heading', "map-ownship")
assert source != before, "the pointless shape is not written the way this case edits it"
open(path, "w", encoding="utf-8").write(source)
PY
if bash "$fixture/scripts/check-web-situation-map.sh" "$fixture" >/dev/null 2>&1; then
    echo "the web situation guard accepted a point where no heading was stated" >&2
    exit 1
fi
cp "$repo_root/clients/web/situation-ownship.js" "$web/"

# In wind the nose and the course differ, and a leader along the nose hides
# the difference a reader is entitled to.
python3 - "$web/situation-ownship.js" <<PY
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
before = source
source = source.replace('leaderEndpoint(position, track)', "leaderEndpoint(position, nose)", 1)
assert source != before, "the leader is not written the way this case edits it"
open(path, "w", encoding="utf-8").write(source)
PY
if bash "$fixture/scripts/check-web-situation-map.sh" "$fixture" >/dev/null 2>&1; then
    echo "the web situation guard accepted a leader drawn along the nose" >&2
    exit 1
fi
cp "$repo_root/clients/web/situation-ownship.js" "$web/"

# A vehicle holding station reports drift whose direction wanders through
# the whole compass; drawn as a course it is a course it is not on.
python3 - "$web/situation-motion.js" <<PY
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
before = source
source = source.replace('TRACK_FLOOR_MPS', "TRACK_FLOOR_NEVER")
assert source != before, "the floor is not written the way this case edits it"
open(path, "w", encoding="utf-8").write(source)
PY
if bash "$fixture/scripts/check-web-situation-map.sh" "$fixture" >/dev/null 2>&1; then
    echo "the web situation guard accepted station-keeping drift as a course" >&2
    exit 1
fi
cp "$repo_root/clients/web/situation-motion.js" "$web/"
# A stub map takes any layer spec, so a suite that never drives the mark
# through real MapLibre cannot tell a drawn course from a rejected one.
python3 - "$web/situation-map.browser.test.mjs" <<PY
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
before = source
source = source.replace("queryRenderedFeatures", "countRenderedFeatures")
assert source != before, "the render readback is not written the way this case edits it"
open(path, "w", encoding="utf-8").write(source)
PY
if bash "$fixture/scripts/check-web-situation-map.sh" "$fixture" >/dev/null 2>&1; then
    echo "the web situation guard accepted a course never drawn in real MapLibre" >&2
    exit 1
fi
cp "$repo_root/clients/web/situation-map.browser.test.mjs" "$web/"
# A gate that names a band either side of unit length, beside a yaw that is
# the rotation's only AT unit length, reads a heading wrong by degrees and
# draws the mark pointed while it does.
python3 - "$web/situation-motion.js" <<PY
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
before = source
source = source.replace(
    "w * w + x * x - y * y - z * z", "1 - 2 * (y * y + z * z)", 1
)
assert source != before, "the yaw is not written the way this case edits it"
open(path, "w", encoding="utf-8").write(source)
PY
if bash "$fixture/scripts/check-web-situation-map.sh" "$fixture" >/dev/null 2>&1; then
    echo "the web situation guard accepted a yaw that assumes an unenforced length" >&2
    exit 1
fi
cp "$repo_root/clients/web/situation-motion.js" "$web/"

# A quaternion that is not near unit length is not a rotation, and a
# truncated frame decodes to zeros that read as a confident due north.
python3 - "$web/situation-motion.js" <<PY
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
before = source
source = source.replace("normSquared < 0.9 || normSquared > 1.1", "false", 1)
assert source != before, "the length gate is not written the way this case edits it"
open(path, "w", encoding="utf-8").write(source)
PY
if bash "$fixture/scripts/check-web-situation-map.sh" "$fixture" >/dev/null 2>&1; then
    echo "the web situation guard accepted a heading read off a degenerate quaternion" >&2
    exit 1
fi
cp "$repo_root/clients/web/situation-motion.js" "$web/"

# The estimate lane advances attitude, velocity and the fix apart, and the
# producer withholds a group only after three seconds. A direction drawn
# from a group that stopped advancing points where the vehicle no longer is.
python3 - "$web/situation-ownship.js" <<PY
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
before = source
source = source.replace("groupIsCurrent", "groupIsIgnored")
assert source != before, "the freshness gate is not written the way this case edits it"
open(path, "w", encoding="utf-8").write(source)
PY
if bash "$fixture/scripts/check-web-situation-map.sh" "$fixture" >/dev/null 2>&1; then
    echo "the web situation guard accepted a direction that outlives its group" >&2
    exit 1
fi
cp "$repo_root/clients/web/situation-ownship.js" "$web/"

# Assigning the whole class list takes the renderer's own marker class with
# it, and the marker's placement is that class's rules.
python3 - "$web/situation-ownship.js" <<PY
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
before = source
source = source.replace(
    'element.classList.toggle("map-ownship-unknown-heading", headingDeg === null);',
    'element.className = headingDeg === null'
    ' ? "map-ownship map-ownship-unknown-heading" : "map-ownship";',
    1,
)
assert source != before, "the shape change is not written the way this case edits it"
open(path, "w", encoding="utf-8").write(source)
PY
if bash "$fixture/scripts/check-web-situation-map.sh" "$fixture" >/dev/null 2>&1; then
    echo "the web situation guard accepted a shape change that wipes the marker class" >&2
    exit 1
fi
cp "$repo_root/clients/web/situation-ownship.js" "$web/"
# An unstamped authorization is a fail-closed case, and the map is the
# first consumer of this mask off the raw wire message.
sed -i.bak 's/authorizedFlags(lane, source)/lane.validFlags ?? 0/' \
    "$web/situation-ownship.js"
reject "a direction drawn on an authorization nobody stamped"
restore clients/web/situation-ownship.js

# A reader reads a direction off a point, so the shape a mark takes when
# no heading is stated has to exist in the stylesheet as well as the module.
sed -i.bak 's/map-ownship-unknown-heading/map-ownship-no-point/g' "$web/index.html"
reject "a stylesheet with no shape for an unstated heading"
restore clients/web/index.html

# The map opens pitched and the reader can turn it. A mark aligned to the
# screen points somewhere the vehicle is not for as long as either holds.
sed -i.bak 's/rotationAlignment: "map"/rotationAlignment: "viewport"/' \
    "$web/situation-ownship.js"
reject "a mark aligned to the viewport rather than the map"
restore clients/web/situation-ownship.js

sed -i.bak 's/pitchAlignment: "map"/pitchAlignment: "viewport"/' \
    "$web/situation-ownship.js"
reject "a mark whose pitch follows the screen rather than the map"
restore clients/web/situation-ownship.js

# The leader is a distance over the ground, so it is drawn in geographic
# coordinates; a fixed pixel length states a different distance at every
# zoom.
sed -i.bak 's/type: "geojson"/type: "image"/' "$web/situation-ownship.js"
reject "a leader that is not drawn in geographic coordinates"
restore clients/web/situation-ownship.js

# A stub map takes any layer spec. Only a real style can refuse one, and
# only a real map can turn under a mark.
sed -i.bak 's/attachOwnship/attachSomethingElse/g' "$web/situation-map.browser.test.mjs"
reject "a browser suite that never drives the mark through real MapLibre"
restore clients/web/situation-map.browser.test.mjs

sed -i.bak 's/transformWhenTurned/transformIgnored/g' "$web/situation-map.browser.test.mjs"
reject "a browser suite that never turns the map under the mark"
restore clients/web/situation-map.browser.test.mjs

# The guardrails only hold while CI runs them.
sed -i.bak '/situation-motion.test.mjs/d' "$fixture/.github/workflows/ci.yml"
reject "a CI workflow that no longer checks the heading and the track"
restore .github/workflows/ci.yml

sed -i.bak 's|run: node clients/web/situation-motion.test.mjs|# &|' \
    "$fixture/.github/workflows/ci.yml"
reject "a commented-out heading and track step"
restore .github/workflows/ci.yml
echo "web situation map guards reject each loss"
