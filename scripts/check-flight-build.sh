#!/usr/bin/env bash
# Gate: the flight build carries no simulation-only code.
#
# The session host builds with --no-default-features for a flight
# deployment. That build must compile, and its dependency tree must not
# contain a simulation-only crate: the simulator adapters, the sidecar
# video client, the XIL truth-oracle bindings, or the tuning campaign.
#
# The tuning crates decide which control-feel calibration ships. They run
# a simulator, search a parameter space and write a journal, and a "SIM /
# NOT FOR FLIGHT" line in a crate doc is a note rather than a gate. One of
# them lives under crates/ rather than tools/, which is where an ordinary
# dependency edge would put it in the flight tree without anyone noticing.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${REPO_ROOT}"

echo "building the flight host (no default features)..."
cargo build -q --release -p pilotage-session-host --no-default-features

FORBIDDEN='pilotage-adapter-gazebo|pilotage-sim-video|pilotage-adapter-reference|aviate-xil-contract|aviate-xil-shm|pilotage-tuning-feedback|flight-tune'
TREE="$(cargo tree -p pilotage-session-host --no-default-features -e normal)"
if grep -qE "${FORBIDDEN}" <<<"${TREE}"; then
  echo "check-flight-build: FAILED - simulation-only crate in the flight tree:" >&2
  grep -E "${FORBIDDEN}" <<<"${TREE}" >&2
  exit 1
fi
echo "check-flight-build: OK"
