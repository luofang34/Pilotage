#!/usr/bin/env bash
# Prove that the ownship wiring guard rejects each way the wire was cut before.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture="$(mktemp -d "${TMPDIR:-/tmp}/pilotage-ownship-wiring.XXXXXX")"
trap 'rm -rf "$fixture"' EXIT

app="$fixture/clients/apple-situation/App"
mkdir -p "$app" "$fixture/scripts"
cp "$repo_root/clients/apple-situation/App/"*.swift "$app/"
cp "$repo_root/scripts/check-situation-ownship-wiring.sh" "$fixture/scripts/"

gate="$fixture/scripts/check-situation-ownship-wiring.sh"
bash "$gate" "$fixture" >/dev/null

# Each case removes one wire and expects a refusal. The message names the fault the
# removal recreates, so a failure here reads as the fault rather than as a broken test.
reject() {
    local message=$1
    if bash "$gate" "$fixture" >/dev/null 2>&1; then
        echo "the ownship wiring guard accepted $message" >&2
        exit 1
    fi
}

restore() {
    cp "$repo_root/clients/apple-situation/App/$1" "$app/$1"
}

sed -i.bak '/model.currentOwnship = /,+9d' "$app/PilotageSituationApp.swift"
reject "a model whose ownship reader is never set"
restore PilotageSituationApp.swift

sed -i.bak '/model.refreshEvidence()/d' "$app/PilotageSituationApp.swift"
reject "an evidence write nothing asks for"
restore PilotageSituationApp.swift

sed -i.bak '/ownship.refreshOrientation()/d' "$app/PilotageSituationApp.swift"
reject "a turned tablet that never reaches the heading orientation"
restore PilotageSituationApp.swift

sed -i.bak '/onChange(of: ownship.heading)/d' "$app/PilotageSituationApp.swift"
reject "a map that turns only when the control is pressed"
restore PilotageSituationApp.swift

sed -i.bak 's/applyFollow(animated: false)/applyFollow(animated: true)/' \
    "$app/PilotageSituationApp.swift"
reject "a camera eased on every reading"
restore PilotageSituationApp.swift

sed -i.bak 's/^        compass.start()$//' "$app/OwnshipPosition.swift"
reject "a position request that leaves the compass stopped"
restore OwnshipPosition.swift

sed -i.bak 's/case .landscapeLeft: .landscapeRight/case .landscapeLeft: .landscapeLeft/' \
    "$app/OwnshipPosition.swift"
reject "an interface orientation passed straight through as a device orientation"
restore OwnshipPosition.swift

sed -i.bak 's/trueHeading >= 0/trueHeading > -999/' "$app/OwnshipPosition.swift"
reject "a true heading trusted before the platform has variation"
restore OwnshipPosition.swift

sed -i.bak 's/@Published var follow/var follow/' "$app/OwnshipPosition.swift"
reject "a follow mode a closure would read as it was rather than as it is"
restore OwnshipPosition.swift

bash "$gate" "$fixture" >/dev/null
echo "ownship wiring guard rejects each cut wire"
