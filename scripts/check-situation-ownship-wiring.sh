#!/usr/bin/env bash
# Keep every ownship seam connected to a caller.
#
# Each fault this guards against was a declaration with nothing on the other end: a
# reader the model offered and the view never set, an evidence write nothing asked for,
# a compass the position request never started. All of them compile, and all of them
# look correct in the file that declares them. Only the call site proves the wire.
set -euo pipefail

root="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
app="$root/clients/apple-situation/App"
status=0

require_pattern() {
    local pattern=$1
    local file=$2
    local message=$3
    if ! grep -Eq "$pattern" "$file"; then
        echo "FORBIDDEN: $message" >&2
        status=1
    fi
}

# A seam must be reached from outside the file that declares it.
#
# Counting uses across the whole directory is not enough: a model that declares a reader
# and reads it in the same file looks wired from every angle except the one that matters,
# which is whether anything ever supplies it. The caller may live in any other file, so
# the rule is "some other file names it", not "a named file names it".
require_caller() {
    local name=$1
    local message=$2
    local declaring
    local outside
    declaring=$(grep -El "(var|func) $name\b" "$app"/*.swift || true)
    outside=$(grep -El "\b$name\b" "$app"/*.swift | grep -Fxv "$declaring" || true)
    if [ -z "$outside" ]; then
        echo "FORBIDDEN: $message" >&2
        status=1
    fi
}

require_caller currentOwnship \
    "the view must give the model a reader for ownship state, or the file it writes cannot describe the run"
require_caller onOwnship \
    "the aircraft's own return must reach the ownship model"
require_caller refreshEvidence \
    "something must ask for an evidence write, or a run with no radio traffic leaves no record"
require_caller startIfPermitted \
    "a reader who already granted permission must not have to press a control before the sensors are asked anything"
require_caller refreshOrientation \
    "a turned tablet must reach the heading orientation, or heading up reads ninety degrees off"

if ! grep -A 2 'func requestPositionIfNeeded()' "$app/OwnshipPosition.swift" \
    | grep -Eq '^\s+start\(\)'; then
    echo "FORBIDDEN: asking for a position must start every sensor a heading needs, not the receiver alone" >&2
    status=1
fi

require_pattern 'compass\.start\(\)' "$app/OwnshipPosition.swift" \
    "the compass must be started, or a heading up map has nothing to turn to"
require_pattern 'locationManagerShouldDisplayHeadingCalibration' "$app/OwnshipPosition.swift" \
    "a compass that cannot trust its reading must be allowed to ask the reader to swing the tablet"
require_pattern 'trueHeading >= 0' "$app/OwnshipPosition.swift" \
    "a true heading is absent until the platform has variation, and the magnetic reading must answer instead"
require_pattern 'case \.landscapeLeft: \.landscapeRight' "$app/OwnshipPosition.swift" \
    "the interface and device orientations name landscape from opposite ends and must be swapped"
require_pattern '@Published var follow' "$app/OwnshipPosition.swift" \
    "the follow mode must live in the model, because a closure that reads it outlives the view value"

# Following is a mode. A camera that only moves when the control is pressed is a jump
# wearing the name of a mode, and it stops the moment the aircraft does anything.
require_pattern 'onChange\(of: ownship\.fix\)' "$app/PilotageSituationApp.swift" \
    "the map must follow the position as it changes, not only when the control is pressed"
require_pattern 'onChange\(of: ownship\.heading\)' "$app/PilotageSituationApp.swift" \
    "the map must turn with the aircraft as the heading changes"
require_pattern 'applyFollow\(animated: false\)' "$app/PilotageSituationApp.swift" \
    "a camera eased on every reading trails the aircraft, so continuous following must not animate"

exit "$status"
