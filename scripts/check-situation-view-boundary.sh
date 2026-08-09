#!/usr/bin/env bash
# Keep the SituationView contract independent from providers and link services.
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root_dir"

metadata="$(cargo metadata --no-deps --format-version 1)"
dependencies="$(jq -r '
  .packages[]
  | select(.name == "pilotage-situation-view")
  | .dependencies[]
  | select(.kind == null or .kind == "normal")
  | .name
' <<<"$metadata")"

status=0
while IFS= read -r dependency; do
    [ -z "$dependency" ] && continue
    case "$dependency" in
        serde|serde_json|thiserror) ;;
        *)
            echo "FORBIDDEN: pilotage-situation-view has direct dependency $dependency" >&2
            status=1
            ;;
    esac
done <<<"$dependencies"

if rg -n -i \
    '\b(aero[_-]?link|avionics[_-]?link|aerocontext|maplibre|gdl90|fis[_-]?b)\b' \
    crates/pilotage-situation-view/src \
    crates/pilotage-situation-view/corpus; then
    echo "FORBIDDEN: SituationView contains a provider or link-service type" >&2
    status=1
fi

if [ "$status" -ne 0 ]; then
    echo "check-situation-view-boundary: FAILED" >&2
    exit 1
fi

echo "check-situation-view-boundary: OK"
