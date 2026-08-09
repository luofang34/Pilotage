#!/bin/sh
# Generate the application and its private AeroLink project copy.
set -eu

client_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
: "${AERO_LINK_HOST_BUNDLE_IDENTIFIER:=org.luofang.pilotage.situation}"
: "${AERO_LINK_DRIVER_BUNDLE_IDENTIFIER:=${AERO_LINK_HOST_BUNDLE_IDENTIFIER}.aerolink-driver}"

case "$AERO_LINK_DRIVER_BUNDLE_IDENTIFIER" in
    "$AERO_LINK_HOST_BUNDLE_IDENTIFIER".*) ;;
    *)
        echo "the driver App ID must begin with the host App ID" >&2
        exit 2
        ;;
esac

export AERO_LINK_HOST_BUNDLE_IDENTIFIER
export AERO_LINK_DRIVER_BUNDLE_IDENTIFIER
sh "$client_root/scripts/prepare-aero-link.sh"
sh "$client_root/.build/aero-link/platforms/apple/scripts/generate-project.sh" --quiet
cd "$client_root"
xcodegen generate --quiet
echo "generated the Pilotage situation project"
