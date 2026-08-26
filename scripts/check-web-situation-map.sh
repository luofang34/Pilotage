#!/usr/bin/env bash
# Verify the web situation map boundary.
#
# One style file drives the Apple renderer and the web renderer. The web
# client must consume the exported copy of the Apple style, keep the
# renderer and the assets as uncommitted build artifacts, boot without
# either, and stay offline at run time.
set -euo pipefail

root="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
web="$root/clients/web"
map_module="$web/situation-map.js"
style_module="$web/situation-style.js"
camera_module="$web/situation-camera.js"
ownship_module="$web/situation-ownship.js"
motion_module="$web/situation-motion.js"
vendor_script="$root/scripts/vendor-maplibre-web.sh"
assets_script="$root/scripts/build-web-situation-assets.sh"
ci="$root/.github/workflows/ci.yml"
status=0

for path in "$map_module" "$style_module" "$camera_module" "$ownship_module" \
    "$web/situation-style.test.mjs" "$web/situation-camera.test.mjs" \
    "$web/situation-ownship.test.mjs" "$motion_module" \
    "$web/situation-motion.test.mjs" \
    "$web/situation-map.browser.test.mjs" "$vendor_script" "$assets_script"; do
    if [ ! -f "$path" ]; then
        echo "FORBIDDEN: required web situation file is missing: $path" >&2
        status=1
    fi
done

if [ "$status" -ne 0 ]; then
    exit 1
fi

# The style is authored once, in the Apple resource tree. A committed copy
# under clients/web would fork it.
if git -C "$root" ls-files -- 'clients/web' | grep -qi 'SituationStyle'; then
    echo "FORBIDDEN: clients/web must not commit a situation style fork" >&2
    status=1
fi
if ! grep -Fq 'clients/apple/Resources' "$assets_script" \
    || ! grep -Fq 'SituationStyle.json' "$assets_script"; then
    echo "FORBIDDEN: the asset export must copy the Apple situation style" >&2
    status=1
fi

# The renderer and the exported assets are build artifacts, and so is the
# export's staging directory.
if ! grep -Eq '^clients/web/vendor/$' "$root/.gitignore" \
    || ! grep -Eq '^clients/web/situation-assets/$' "$root/.gitignore" \
    || ! grep -Eq '^clients/web/situation-assets\.new/$' "$root/.gitignore"; then
    echo "FORBIDDEN: vendor and situation assets must stay build artifacts" >&2
    status=1
fi
if git -C "$root" ls-files -- 'clients/web/vendor' 'clients/web/situation-assets' \
    'clients/web/situation-assets.new' | grep -q .; then
    echo "FORBIDDEN: a vendored or exported file is committed" >&2
    status=1
fi

# The renderer archive is pinned by version and digest.
if ! grep -Eq '^MAPLIBRE_VERSION="[0-9]+\.[0-9]+\.[0-9]+"$' "$vendor_script" \
    || ! grep -Eq '^MAPLIBRE_TARBALL_SHA256="[0-9a-f]{64}"$' "$vendor_script"; then
    echo "FORBIDDEN: the renderer vendor must pin a version and a digest" >&2
    status=1
fi

# Viewer boot must complete without the vendor directory: the renderer
# import stays dynamic, inside the map module, behind the stage selection.
# The from-clause grep also catches a static import split over lines.
if grep -Eq '^import[^(]*vendor/' "$map_module" \
    || grep -Eq 'from[[:space:]]*["'\''][^"'\'']*vendor/' "$map_module"; then
    echo "FORBIDDEN: the renderer must load through a dynamic import" >&2
    status=1
fi
if ! grep -Fq 'await import(VENDOR_MODULE)' "$map_module"; then
    echo "FORBIDDEN: the map module must import the vendored renderer lazily" >&2
    status=1
fi
if grep -Eq 'vendor/' "$web/index.html" "$web/main.js" "$web/layout.js" \
    "$style_module"; then
    echo "FORBIDDEN: boot-path files must not reference the vendor directory" >&2
    status=1
fi

# The web client loads no external resource at run time. The second
# pattern catches a protocol-relative URL.
if grep -Eqi 'https?://' "$map_module" "$style_module" "$camera_module" \
    || grep -Eq '["'\'']//' "$map_module" "$style_module" "$camera_module"; then
    echo "FORBIDDEN: the situation modules must have no network URL" >&2
    status=1
fi

# A reader who turned or tilted the map needs one way back to north and one
# way back to straight down, and the pointer needs a control to reach a
# camera that touch reaches with two fingers.
if ! grep -Fq 'NavigationControl' "$map_module" \
    || ! grep -Fq 'visualizePitch: true' "$map_module"; then
    echo "FORBIDDEN: a pointer must be able to turn and tilt the map" >&2
    status=1
fi
# The thresholds and the wording are the Apple client's; the web client
# must read them from the shared vocabulary rather than restate them.
if grep -Eq '[<>]=?[[:space:]]*0\.5|Facing north|turn back to north' "$map_module"; then
    echo "FORBIDDEN: camera thresholds and wording belong to situation-camera.js" >&2
    status=1
fi

# The closest zoom follows the terrain manifest, exactly as on the Apple
# client, rather than being written twice.
if ! grep -Fq 'maxZoom: deriveMaximumZoom' "$map_module"; then
    echo "FORBIDDEN: the map must derive its closest zoom from the manifest" >&2
    status=1
fi

# An unavailable map states a typed reason (ADR-0037), never an empty map.
for reason in MAP_LIBRARY_MISSING MAP_ASSETS_MISSING MAP_STYLE_INVALID \
    MAP_RENDER_FAILED; do
    if ! grep -Fq "$reason" "$map_module"; then
        echo "FORBIDDEN: the map module must report the $reason state" >&2
        status=1
    fi
done

# The vehicle mark states a typed reason for its absence too, and a mark
# that stops being refreshed is withdrawn on a clock of its own: a link
# that goes silent delivers no sample to notice the silence with.
for reason in OWNSHIP_NO_TELEMETRY OWNSHIP_NO_FIX; do
    if ! grep -Fq "$reason" "$ownship_module"; then
        echo "FORBIDDEN: the ownship module must report the $reason state" >&2
        status=1
    fi
done
# Two lanes can carry a position, and a reader has to be told which one is
# under the mark: an oracle is exact by construction and an estimate is a
# solution with an accuracy of its own.
for source in simulation-truth operational-estimate; do
    if ! grep -Fq "$source" "$ownship_module"; then
        echo "FORBIDDEN: the ownship module must name the $source lane" >&2
        status=1
    fi
done
if ! grep -Fq 'ownshipSource' "$ownship_module"; then
    echo "FORBIDDEN: the mark must say which measurement is under it" >&2
    status=1
fi

if ! grep -Fq 'OWNSHIP_STALE_AFTER_MS' "$ownship_module" \
    || ! grep -Eq 'return \{ observe, age, marker \}' "$ownship_module"; then
    echo "FORBIDDEN: the vehicle mark must be withdrawn when its fix stops arriving" >&2
    status=1
fi
# A clock nothing turns is a rule nothing enforces, so the check is on the
# call that turns it and not on the name being mentioned somewhere.
# The clock is armed at wiring time and re-armed when a restored page
# resumes, so both calls are required: one of them alone leaves either a
# mark that never ages or a mark that stops ageing after a navigation.
if [ "$(grep -c 'setInterval(ageOwnship, OWNSHIP_AGE_INTERVAL_MS)' "$map_module")" != "2" ]; then
    echo "FORBIDDEN: something must drive the mark's own clock" >&2
    status=1
fi
# The back/forward cache stops a page and later resumes it with its
# telemetry intact. A clock stopped without re-arming leaves the mark
# updating and never ageing for the rest of the page's life.
if ! grep -Fq 'pageshow' "$map_module" || ! grep -Fq 'visibilitychange' "$map_module"; then
    echo "FORBIDDEN: the mark's clock must survive a restored page and a hidden tab" >&2
    status=1
fi

# The mark turns to a heading the vehicle STATED. A pointed shape that
# never turned would assert screen-up on a map the reader can rotate, and a
# shape rotated from an unstated attitude asserts due north.
if ! grep -Fq 'marker.setRotation(headingDeg ?? 0)' "$ownship_module"; then
    echo "FORBIDDEN: the mark must turn to the heading the vehicle states" >&2
    status=1
fi
if ! grep -Fq 'map-ownship-unknown-heading' "$ownship_module" \
    || ! grep -Fq 'map-ownship-unknown-heading' "$web/index.html"; then
    echo "FORBIDDEN: a mark with no stated heading must have no point in it" >&2
    status=1
fi
# Both alignments are to the map. The map opens pitched and turns under the
# reader, and a mark aligned to the screen points where the vehicle is not.
for alignment in rotationAlignment pitchAlignment; do
    if ! grep -Eq "$alignment: \"map\"" "$ownship_module"; then
        echo "FORBIDDEN: the mark must be aligned to the map, not the viewport" >&2
        status=1
    fi
done

# Heading is where the nose points and track is where the vehicle goes.
# They differ in wind, and a leader drawn along the heading hides that.
if ! grep -Fq 'leaderEndpoint(position, track)' "$ownship_module"; then
    echo "FORBIDDEN: the leader must follow the track and not the nose" >&2
    status=1
fi
# The leader is a distance over the ground, so it is drawn in geographic
# coordinates: a fixed pixel length states a different distance at every
# zoom, and the line means "where the vehicle arrives in a minute".
if ! grep -Fq '"type": "geojson"' "$ownship_module" \
    && ! grep -Fq 'type: "geojson"' "$ownship_module"; then
    echo "FORBIDDEN: the leader must be drawn in geographic coordinates" >&2
    status=1
fi
# A quaternion that is not near unit length is not a rotation, and a
# vehicle holding station is on no course. Both refusals are what keeps the
# mark from stating a direction nobody measured.
if ! grep -Fq 'TRACK_FLOOR_MPS' "$motion_module" \
    || ! grep -Fq 'normSquared < 0.9 || normSquared > 1.1' "$motion_module"; then
    echo "FORBIDDEN: an unmeasured direction must not be drawn as one" >&2
    status=1
fi
# The gate passes a band either side of unit length, so the yaw must come
# out of a form that is exact at any scale. The `1 - 2(...)` form is the
# rotation's only at exactly unit length, and inside the band it reads a
# heading wrong by degrees with the mark drawn pointed.
if grep -Fq '1 - 2 * (y * y + z * z)' "$motion_module"; then
    echo "FORBIDDEN: the yaw must not assume a unit length the gate does not enforce" >&2
    status=1
fi
# The mask that authorizes a direction, and the quality beside it, mean
# nothing without the status observation backing them; absence is a
# fail-closed case, and the map reads this mask off the raw wire message
# rather than through the ingress that applies the gate for the panels.
if ! grep -Fq 'authorizedFlags(lane, source)' "$ownship_module" \
    || ! grep -Fq 'estimatorStatusStamp' "$ownship_module" \
    || ! grep -Fq 'QUALITY_UNUSABLE' "$ownship_module"; then
    echo "FORBIDDEN: a direction must not be drawn on an authorization nobody stamped" >&2
    status=1
fi

# A direction is drawn only while its own group keeps producing
# measurements. The estimate lane stamps attitude, velocity and the fix
# apart and advances them apart.
if ! grep -Fq 'GROUP_COHERENCE_MS' "$ownship_module" \
    || ! grep -Fq 'groupIsCurrent' "$ownship_module"; then
    echo "FORBIDDEN: a direction must not outlive the group that measured it" >&2
    status=1
fi
# Assigning the whole class list takes the renderer's own marker class with
# it, and the marker's placement is that class's rules.
if grep -Eq 'element\.className *=' "$ownship_module" \
    && [ "$(grep -Ec 'element\.className *=' "$ownship_module")" -gt 1 ]; then
    echo "FORBIDDEN: the mark's shape must change by class, not by class list" >&2
    status=1
fi

# The unit suite drives the mark against a stub map, which takes any source
# and layer it is handed. Only a real style can refuse one, and only a real
# map can turn under a mark, so the browser suite must attach the mark to a
# real map and read back both the line on screen and the turned mark.
browser_test="$web/situation-map.browser.test.mjs"
if ! grep -Fq 'attachOwnship' "$browser_test" \
    || ! grep -Fq 'queryRenderedFeatures' "$browser_test" \
    || ! grep -Fq 'transformWhenTurned' "$browser_test"; then
    echo "FORBIDDEN: the course and the turned mark must be proven in real MapLibre" >&2
    status=1
fi

# The guardrails only hold while CI runs them. The step must sit on a
# live run: line; a commented-out step does not count.
for step in 'clients/web/situation-style.test.mjs' \
    'clients/web/situation-camera.test.mjs' \
    'clients/web/situation-ownship.test.mjs' \
    'clients/web/situation-motion.test.mjs' \
    'clients/web/situation-map.browser.test.mjs' \
    'scripts/vendor-maplibre-web.sh' \
    'scripts/check-web-situation-map.sh' \
    'scripts/test-check-web-situation-map.sh'; do
    if ! grep -E '^[[:space:]]*run:' "$ci" | grep -Fq "$step"; then
        echo "FORBIDDEN: CI must run $step" >&2
        status=1
    fi
done

if [ "$status" -ne 0 ]; then
    echo "Web situation map: FAILED" >&2
    exit 1
fi

echo "Web situation map: OK"
