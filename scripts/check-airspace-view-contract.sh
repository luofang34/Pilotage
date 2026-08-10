#!/usr/bin/env bash
# Keep the AirspaceView resolver stateless, source-neutral, and map-independent.
set -euo pipefail

root="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
crate="$root/crates/pilotage-airspace-view"
model="$crate/src/model.rs"
resolver="$crate/src/resolve.rs"
contract="$root/docs/airspace-view-resolution-contract.md"
status=0

require_text() {
    local pattern="$1"
    local file="$2"
    local message="$3"
    if [ ! -f "$file" ] || ! grep -qF "$pattern" "$file"; then
        echo "FORBIDDEN: $message" >&2
        status=1
    fi
}

if [ -d "$crate" ] && grep -RInE 'MapLibre|MLN[A-Z]|UIKit|SwiftUI|GeoJSON' "$crate"; then
    echo "FORBIDDEN: AirspaceView names a display implementation" >&2
    status=1
fi

require_text 'pub struct AirspaceViewV1;' "$resolver" \
    'AirspaceView must remain a stateless derived resolver'
require_text 'pub struct SubjectIdentityV1' "$model" \
    'a stable subject identifier must retain its Navdata cycle'
require_text 'IdentifierFromAnotherCycle' "$model" \
    'cycle mismatch must remain a typed resolution failure'
require_text 'PartialGeometryNotCarried' "$model" \
    'partial geometry must fail explicitly when the baseline cannot supply it'
require_text 'DirectGeometryExtentMismatch' "$model" \
    'direct geometry must not enlarge a partial subject'
require_text 'SupplementalOnly' "$model" \
    'the result must state that a map is supplemental'
require_text 'The resolver does not enlarge a partial subject to a complete subject.' "$contract" \
    'the partial-subject rule is missing from the contract'
require_text 'An empty map does not mean that no update applies.' "$contract" \
    'the contract does not require a complete non-map surface'

if [ "$status" -ne 0 ]; then
    echo "AirspaceView contract: FAILED" >&2
    exit 1
fi

echo "AirspaceView contract: OK"
