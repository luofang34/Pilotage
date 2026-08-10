#!/bin/sh
# Generate the application and its private AeroLink project copy.
set -eu

client_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
: "${AERO_LINK_HOST_BUNDLE_IDENTIFIER:=org.luofang.pilotage.situation}"
: "${AERO_LINK_DRIVER_BUNDLE_IDENTIFIER:=${AERO_LINK_HOST_BUNDLE_IDENTIFIER}.aerolink-driver}"

case "${PILOTAGE_MAPLIBRE_TERRAIN:-0}" in
    0) PILOTAGE_MAPLIBRE_SWIFT_CONDITIONS='' ;;
    1)
        PILOTAGE_MAPLIBRE_SWIFT_CONDITIONS=PILOTAGE_MAPLIBRE_TERRAIN
        sh "$client_root/scripts/build-maplibre-terrain.sh"
        ;;
    *)
        echo "PILOTAGE_MAPLIBRE_TERRAIN must be 0 or 1" >&2
        exit 2
        ;;
esac

case "$AERO_LINK_DRIVER_BUNDLE_IDENTIFIER" in
    "$AERO_LINK_HOST_BUNDLE_IDENTIFIER".*) ;;
    *)
        echo "the driver App ID must begin with the host App ID" >&2
        exit 2
        ;;
esac

export AERO_LINK_HOST_BUNDLE_IDENTIFIER
export AERO_LINK_DRIVER_BUNDLE_IDENTIFIER
export PILOTAGE_MAPLIBRE_SWIFT_CONDITIONS
sh "$client_root/scripts/prepare-aero-link.sh"
sh "$client_root/.build/aero-link/platforms/apple/scripts/generate-project.sh" --quiet
cd "$client_root"
xcodegen generate --quiet
echo "generated the Pilotage situation project"
