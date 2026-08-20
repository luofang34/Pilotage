#!/usr/bin/env bash
# Gate: the flight build carries no simulation-only code.
#
# The session host builds with --no-default-features for a flight
# deployment. That build must compile, and its dependency tree must not
# contain a simulation-only crate: the simulator adapters, the sidecar
# video client, or the XIL truth-oracle bindings.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${REPO_ROOT}"

echo "building the flight host (no default features)..."
cargo build -q --release -p pilotage-session-host --no-default-features

FORBIDDEN='pilotage-adapter-gazebo|pilotage-sim-video|pilotage-adapter-reference|aviate-xil-contract|aviate-xil-shm'
TREE="$(cargo tree -p pilotage-session-host --no-default-features -e normal)"
if grep -qE "${FORBIDDEN}" <<<"${TREE}"; then
  echo "check-flight-build: FAILED - simulation-only crate in the flight tree:" >&2
  grep -E "${FORBIDDEN}" <<<"${TREE}" >&2
  exit 1
fi
echo "check-flight-build: OK"
