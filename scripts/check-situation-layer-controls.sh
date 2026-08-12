#!/usr/bin/env bash
# Keep situation layer and traffic detail policy in portable Rust.
set -euo pipefail

root="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
portable="$root/crates/pilotage-presentation/src"
ffi="$root/clients/apple/rust/pilotage-situation-ffi/src/session.rs"
app="$root/clients/apple/App"
binding="$root/clients/apple/Packages/PilotageMapLibreBinding/Sources"
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

require_pattern 'pub struct LayerControl' "$portable/layer.rs" \
    "portable Rust must own layer controls"
require_pattern 'pub struct SourceObservation' "$portable/layer.rs" \
    "portable Rust must own source facts"
require_pattern 'does not mean clear weather' "$portable/layer.rs" \
    "an absent weather source must not imply clear weather"
require_pattern 'pub layer_id: String' "$portable/model.rs" \
    "display features must use an extensible application layer identity"
require_pattern 'pub positionless_traffic:' "$portable/model.rs" \
    "the display batch must carry positionless traffic"
require_pattern 'pub traffic_details:' "$portable/model.rs" \
    "the display batch must carry traffic detail"
require_pattern 'traffic_tracks:' "$portable/policy.rs" \
    "the adapter must retain complete traffic tracks"
require_pattern 'is_enabled[(]TRAFFIC_LAYER_ID[)]' "$portable/policy.rs" \
    "traffic visibility must be portable policy"
require_pattern 'is_enabled[(]WEATHER_REPORT_LAYER_ID[)]' "$portable/policy.rs" \
    "weather visibility must be portable policy"
require_pattern '[.]age_micros[(]now_micros[)]' "$portable/detail.rs" \
    "traffic detail must use each field age"
require_pattern 'source_label[(]timed[.]provenance[)]' "$portable/detail.rs" \
    "traffic detail must use each field provenance"
require_pattern 'absence_reason:' "$portable/detail.rs" \
    "traffic detail must state why a field is absent"
require_pattern 'disabled_traffic_retains_the_newest_state_without_replay' \
    "$portable/tests/traffic.rs" \
    "a test must prove disabled traffic retains current state"
require_pattern 'positionless_track_has_a_list_item' "$portable/tests/traffic.rs" \
    "a test must prove positionless traffic is listed"
require_pattern 'pub fn observe_sources' "$ffi" \
    "the facade must send raw source facts to portable Rust"
require_pattern 'pub fn set_layer_enabled' "$ffi" \
    "the facade must send layer controls to portable Rust"
# These say the reader can reach each control, not which screen holds it. The map face
# and the drawer both belong to the application, and moving a control between them is a
# layout decision; removing one is a capability decision and is what this refuses.
require_view() {
    local view=$1
    local message=$2
    if ! grep -RqE "\<$view\>" "$app"; then
        echo "FORBIDDEN: $message" >&2
        status=1
    fi
}

require_view 'LayerControlsView' "the application must show layer controls"
require_view 'PositionlessTrafficView' "the application must show positionless traffic"
require_view 'TrafficDetailView' "the application must show traffic detail"
require_pattern 'visibleFeatures[(]' \
    "$binding/PilotageMapLibreBinding/SituationMapView.swift" \
    "the map binding must identify a tapped rendered feature"
require_pattern 'onFeatureTapped[?][(]identifier[)]' \
    "$binding/PilotageMapLibreBinding/SituationMapView.swift" \
    "the map binding must return only the stable feature identity"

if grep -RInE \
    '"(terrain-base|traffic|weather-reports|weather-advisories)"|TrackSnapshot|TimedField|Pressure altitude|Transponder code' \
    "$binding"; then
    echo "FORBIDDEN: the map binding contains domain or layer policy" >&2
    status=1
fi

if [ "$status" -ne 0 ]; then
    echo "situation layer controls: FAILED" >&2
    exit 1
fi

echo "situation layer controls: OK"
