#!/usr/bin/env bash
# Resets the flight demo between test iterations: puts the Gazebo world
# back to its initial pose and restarts the FC binary (whose estimator
# state must reset with the vehicle). Host and browser stay up — the shm
# link reattaches by itself.
#
# Usage: scripts/reset-flight-sim.sh [world-name]  (default: default)
set -euo pipefail
export PATH="/opt/homebrew/bin:$PATH"
export GZ_IP="${GZ_IP:-127.0.0.1}"
WORLD="${1:-default}"

echo "resetting world '${WORLD}'..."
gz service -s "/world/${WORLD}/control" \
  --reqtype gz.msgs.WorldControl --reptype gz.msgs.Boolean \
  --timeout 3000 --req 'reset: {all: true}'

echo "restarting FC..."
pkill -9 -f sitl-gazebo-x500 2>/dev/null || true

# When `cargo xtask sim` supervises the session, the supervisor restarts
# the flight controller itself; a script-spawned second FC would fight it
# over the shm writer lease.
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SUPERVISOR_PID_FILE="${REPO_ROOT}/target/xtask-sim/supervisor.pid"
if [[ -f "${SUPERVISOR_PID_FILE}" ]] && kill -0 "$(cat "${SUPERVISOR_PID_FILE}")" 2>/dev/null; then
  echo "done — the xtask supervisor restarts the FC; re-arm from the browser once it logs ready"
  exit 0
fi

sleep 1
AVIATE_DIR="${AVIATE_DIR:-$HOME/Aviate}"
nohup "${AVIATE_DIR}/target/debug/sitl-gazebo-x500" > /tmp/fc_manual.log 2>&1 &
echo "done — re-arm from the browser once the FC logs Ready (~5 s)"
