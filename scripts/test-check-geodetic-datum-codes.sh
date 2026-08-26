#!/usr/bin/env bash
# Prove that the geodetic datum guard rejects each way the readers of a fix
# can fall out of step with each other.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture="$(mktemp -d "${TMPDIR:-/tmp}/pilotage-datum.XXXXXX")"
trap 'rm -rf "$fixture"' EXIT

mkdir -p \
    "$fixture/crates/pilotage-geo/src" \
    "$fixture/schemas/pilotage/v1" \
    "$fixture/clients/web" \
    "$fixture/clients/web-instruments/src/decode_envelope" \
    "$fixture/adapters/aviate/src/adapter" \
    "$fixture/tools/xtask/src/backend" \
    "$fixture/hosts/session-host/src/runtime/engine_actor" \
    "$fixture/scripts" \
    "$fixture/.github/workflows"

datum="$fixture/crates/pilotage-geo/src/datum.rs"
proto="$fixture/schemas/pilotage/v1/telemetry.proto"
decoder="$fixture/clients/web/wire.js"
mapper="$fixture/hosts/session-host/src/runtime/engine_actor/telemetry.rs"
wasm_decoder="$fixture/clients/web-instruments/src/decode_envelope/groups.rs"
attachments="$fixture/adapters/aviate/src/adapter/sim_attachments.rs"
camera="$fixture/adapters/aviate/src/adapter/camera.rs"
launcher="$fixture/tools/xtask/src/backend/aviate_gz.rs"
ci="$fixture/.github/workflows/ci.yml"

cp "$root/crates/pilotage-geo/src/datum.rs" "$datum"
cp "$root/schemas/pilotage/v1/telemetry.proto" "$proto"
cp "$root/clients/web/wire.js" "$decoder"
cp "$root/hosts/session-host/src/runtime/engine_actor/telemetry.rs" "$mapper"
cp "$root/clients/web-instruments/src/decode_envelope/groups.rs" "$wasm_decoder"
cp "$root/adapters/aviate/src/adapter/sim_attachments.rs" "$attachments"
cp "$root/adapters/aviate/src/adapter/camera.rs" "$camera"
cp "$root/tools/xtask/src/backend/aviate_gz.rs" "$launcher"
cp "$root/.github/workflows/ci.yml" "$ci"
cp "$root/scripts/check-geodetic-datum-codes.sh" "$fixture/scripts/"

gate="$fixture/scripts/check-geodetic-datum-codes.sh"
bash "$gate" "$fixture" >/dev/null

reject() {
    if bash "$gate" "$fixture" >/dev/null 2>&1; then
        echo "the geodetic datum guard accepted $1" >&2
        exit 1
    fi
}

restore() {
    case "$1" in
        datum) cp "$root/crates/pilotage-geo/src/datum.rs" "$datum" ;;
        proto) cp "$root/schemas/pilotage/v1/telemetry.proto" "$proto" ;;
        decoder) cp "$root/clients/web/wire.js" "$decoder" ;;
        mapper) cp "$root/hosts/session-host/src/runtime/engine_actor/telemetry.rs" "$mapper" ;;
        ci) cp "$root/.github/workflows/ci.yml" "$ci" ;;
        wasm) cp "$root/clients/web-instruments/src/decode_envelope/groups.rs" "$wasm_decoder" ;;
        attachments) cp "$root/adapters/aviate/src/adapter/sim_attachments.rs" "$attachments" ;;
        camera) cp "$root/adapters/aviate/src/adapter/camera.rs" "$camera" ;;
    esac
}

# A datum the crate can produce that the proto never names is a code a
# reader has no meaning for.
python3 - "$datum" <<'PY'
import re
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
source = source.replace(
    "            3 => Some(Self::Itrf2014),",
    "            3 => Some(Self::Itrf2014),\n            4 => Some(Self::Etrs89),",
    1,
)
open(path, "w", encoding="utf-8").write(source)
PY
reject "a horizontal datum the proto does not name"
restore datum

python3 - "$decoder" <<'PY'
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
source = source.replace("horizontalDatum > 3", "horizontalDatum > 99", 1)
open(path, "w", encoding="utf-8").write(source)
PY
reject "a browser that accepts a datum code the crate cannot produce"
restore decoder

python3 - "$mapper" <<'PY'
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
source = source.replace(
    "horizontal_datum: u32::from(position.horizontal_datum.to_u8()),",
    "horizontal_datum: position.horizontal_datum as u32,",
    1,
)
open(path, "w", encoding="utf-8").write(source)
PY
reject "a datum cast onto the wire instead of converted"
restore mapper

python3 - "$ci" <<'PY'
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
source = source.replace(
    "run: node clients/web/geodetic-fix.test.mjs",
    "# run: node clients/web/geodetic-fix.test.mjs",
    1,
)
open(path, "w", encoding="utf-8").write(source)
PY
reject "a CI workflow whose geodetic test is commented out"
restore ci

# The browser reads telemetry through the wasm decoder. A lane only the
# JavaScript decoder knows reaches no reader, and no decode error says so.
python3 - "$wasm_decoder" <<PY
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
before = source
source = source.replace('        geodetic,\n        geodetic_stamp,\n', "", 1)
assert source != before, "the estimate lane is not written the way this case edits it"
open(path, "w", encoding="utf-8").write(source)
PY
reject "a client decoder that does not surface the estimator's fix"
restore wasm

python3 - "$wasm_decoder" <<PY
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
before = source
source = source.replace("fix.longitude_deg < 180.0", "fix.longitude_deg < 1e9", 1)
assert source != before, "the longitude refusal is not written the way this case edits it"
open(path, "w", encoding="utf-8").write(source)
PY
reject "a client decoder that wraps a longitude the producer did not normalize"
restore wasm

python3 - "$mapper" <<PY
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
before = source
source = source.replace("position.normalized()", "position.validate()", 1)
assert source != before, "the mapping does not normalize the way this case edits it"
open(path, "w", encoding="utf-8").write(source)
PY
reject "a mapping that re-checks a position and then sends its own copy"
restore mapper

python3 - "$mapper" <<PY
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
before = source
source = source.replace("fix.stamp.role == SourceRole::OperationalEstimate", "fix.stamp.role == SourceRole::FcState", 1)
assert source != before, "the estimate role gate is not written the way this case edits it"
open(path, "w", encoding="utf-8").write(source)
PY
reject "a producer that does not gate the estimate lane on the estimate role"
restore mapper

# Each lane gates on its own role before the mapping. A truth lane that
# accepted an estimate-stamped fix would publish bytes the reader drops,
# and the producer would believe it had published a position.
python3 - "$mapper" <<PY
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
before = source
source = source.replace('fix.stamp.role == SourceRole::SimulationTruth', "true", 1)
assert source != before, 'the truth-lane gate is not written the way this case edits it'
open(path, "w", encoding="utf-8").write(source)
PY
reject 'a producer that does not gate the truth lane on the truth role'
restore mapper

# A lane that stops calling the mapping stops re-running the contract, and
# an assembled position then reaches the wire unchecked.
python3 - "$mapper" <<PY
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
before = source
source = source.replace('.and_then(geodetic_to_wire);', ";", 1)
assert source != before, 'the mapping call is not written the way this case edits it'
open(path, "w", encoding="utf-8").write(source)
PY
reject 'a lane that reaches the wire without the mapping'
restore mapper

# Deleting the join leaves every unit test green — they exercise the join
# function directly — while the map states no position for the rest of the
# session, with nothing anywhere to say the feature was removed.
python3 - "$attachments" <<PY
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
before = source
source = source.replace('sample.geodetic = self.paired_fix(sample.stamp);', "", 1)
assert source != before, 'the join is not written the way this case edits it'
open(path, "w", encoding="utf-8").write(source)
PY
reject 'a truth sample that drops the fix the sensor paired with it'
restore attachments

# gz-transport accepts a subscription to a topic nobody publishes, so a
# name that does not match produces no error, no fix, and no diagnostic.
python3 - "$camera" <<PY
import sys
path = sys.argv[1]
source = open(path, encoding="utf-8").read()
before = source
source = source.replace('sensor/navsat_sensor/navsat', "sensor/typo/navsat", 1)
assert source != before, 'the sensor topic is not written the way this case edits it'
open(path, "w", encoding="utf-8").write(source)
PY
reject 'a sensor topic that names no sensor the model publishes'
restore camera

bash "$gate" "$fixture" >/dev/null
echo "geodetic datum guards reject each loss"
