#!/usr/bin/env bash
# Keep every ownship seam connected to a caller.
#
# Each fault this guards against was a declaration with nothing on the other end: a
# reader the model offered and the view never set, an evidence write nothing asked for,
# a compass the position request never started. All of them compile, and all of them
# look correct in the file that declares them. Only the call site proves the wire.
set -euo pipefail

root="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
app="$root/clients/apple/App"
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
require_pattern 'locationManagerShouldDisplayHeadingCalibration' "$app/DeviceSensors.swift" \
    "a compass that cannot trust its reading must be allowed to ask the reader to swing the tablet"
require_pattern 'trueHeading >= 0' "$app/DeviceSensors.swift" \
    "a true heading is absent until the platform has variation, and the magnetic reading must answer instead"
require_pattern 'case \.landscapeLeft: \.landscapeRight' "$app/DeviceSensors.swift" \
    "the interface and device orientations name landscape from opposite ends and must be swapped"
require_pattern '@Published var follow' "$app/OwnshipPosition.swift" \
    "the follow mode must live in the model, because a closure that reads it outlives the view value"

# A glass container blends only the shapes that sit within its spacing. Give it less than
# the gap between the controls and each one becomes its own island: it stops growing out of
# the group and starts arriving from nowhere, which is a one-token change with no error.
require_pattern 'GlassEffectContainer\(spacing: Metrics\.controlSpacing\)' "$app/MapControlsView.swift" \
    "the glass blend distance must equal the gap between controls, or a control cannot morph out of the group"

# A view inserted alongside its parent is covered by the parent's transition and never
# runs its own. Only a change made in a later cycle is an insertion the label can animate,
# so the wait is load bearing and reads like a decoration.
if ! grep -A 6 'levelLabelShown = false' "$app/MapControlsView.swift" | grep -q 'Task.sleep'; then
    echo "FORBIDDEN: the label must change state in a later cycle than the control it sits in, or its own transition never runs" >&2
    status=1
fi

# Following is a mode. A camera that only moves when the control is pressed is a jump
# wearing the name of a mode, and it stops the moment the aircraft does anything.
require_pattern 'onChange\(of: ownship\.fix\)' "$app/SituationContentView.swift" \
    "the map must follow the position as it changes, not only when the control is pressed"
require_pattern 'onChange\(of: ownship\.heading\)' "$app/SituationContentView.swift" \
    "the map must turn with the aircraft as the heading changes"
require_pattern 'applyFollow\(animated: false\)' "$app/SituationContentView.swift" \
    "a camera eased on every reading trails the aircraft, so continuous following must not animate"

# Traffic moves between reports and the display only redraws when something asks it to.
# A record arriving is the one instant at which a projection has nothing to add, so a
# beat that nothing starts leaves every target standing still until the next report.
require_pattern 'projectionTask = Task' "$app/SituationClientModel.swift" \
    "something must drive the beat that redraws traffic between reports"
require_pattern 'projectionTask\?\.cancel\(\)' "$app/SituationClientModel.swift" \
    "the beat must stop with the radio, or a suspended client keeps redrawing"

# The beat must ask the engine again. Republishing the batch already held advances
# nothing, and it looks identical from every angle except the map.
if ! grep -A 20 'func projectionLoop()' "$app/SituationClientModel.swift" \
    | grep -q 'session\.currentDisplay'; then
    echo "FORBIDDEN: the beat must ask the engine where the traffic is now, not republish the last batch" >&2
    status=1
fi

# A bounded guess is refused silently: every refusal leaves the reported position in
# place and reports nothing. Counting the advanced marks is how a run on hardware proves
# the projection fires at all.
require_pattern 'positionIsExtrapolated' "$app/SituationEvidence.swift" \
    "the evidence must count the marks the engine advanced, or a projection that never fires looks the same as one that does"

# The staleness clock must be timed from when a position was MEASURED, not from
# when one arrived.
#
# A host relaying a block whose avionics have stopped keeps delivering samples
# indefinitely. A mark whose clock is refreshed on arrival therefore never goes
# stale however long the vehicle has been silent — which is the single case the
# staleness bound exists to cover. The link states whether the position
# advanced; the clock has to be conditioned on it.
require_pattern 'if vehicle\.fixAdvanced' "$app/OwnshipPosition.swift" \
    "the vehicle staleness clock must be conditioned on fixAdvanced, or a frozen feed keeps the mark alive forever"

# And in one place only. A second, unconditional write refreshes the clock on
# arrival again while the conditional above it still reads correct.
if [ "$(grep -c 'vehicleFixAt = Date()' "$app/OwnshipPosition.swift")" -ne 1 ]; then
    echo "FORBIDDEN: the vehicle staleness clock is set in more than one place, so one of them is not waiting for a new measurement" >&2
    status=1
fi

exit "$status"
