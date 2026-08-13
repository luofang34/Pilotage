#!/bin/sh
# Fetch what the situation client is drawing from a paired device.
#
# The client writes the counts behind its map to Documents/situation-evidence.json, so a
# run can be read without a photograph of the iPad: which receivers report, how many
# features each layer holds, how many shapes the renderer raises, and between which
# heights.
#
# Usage: collect-situation-evidence.sh <device> [destination]
set -eu

device=${1:?usage: collect-situation-evidence.sh <device> [destination]}
destination=${2:-situation-evidence.json}
bundle_identifier=${AERO_LINK_HOST_BUNDLE_IDENTIFIER:-org.luofang.pilotage}

xcrun devicectl device copy from \
    --device "$device" \
    --domain-type appDataContainer \
    --domain-identifier "$bundle_identifier" \
    --source Documents/situation-evidence.json \
    --destination "$destination" >/dev/null

cat "$destination"
