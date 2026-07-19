#!/usr/bin/env bash
# Resets the PX4 flight demo between test iterations: puts the Gazebo
# world back to its initial pose and restarts the px4 binary (whose
# estimator state must reset with the vehicle). Host and browser stay
# up — the MAVLink link detects the restarted stream as a new source
# epoch and the adapter's reset latch clears on fresh telemetry plus
# neutral input.
#
# Usage: scripts/reset-px4-sim.sh [world-name]  (default: default)
set -euo pipefail
export PATH="/opt/homebrew/bin:$PATH"
export GZ_IP="${GZ_IP:-127.0.0.1}"
WORLD="${1:-default}"

echo "resetting world '${WORLD}'..."
gz service -s "/world/${WORLD}/control" \
  --reqtype gz.msgs.WorldControl --reptype gz.msgs.Boolean \
  --timeout 3000 --req 'reset: {all: true}'

echo "restarting PX4..."
# Match ONLY this checkout's SITL binary: a bare "bin/px4" pattern
# would kill unrelated PX4 sessions on the machine.
PX4_DIR="${PX4_DIR:-$HOME/PX4-Autopilot}"
pkill -9 -f "${PX4_DIR}/build/px4_sitl_default/bin/px4" 2>/dev/null || true

# When `cargo xtask sim` supervises the session, the supervisor restarts
# the flight-controller stage itself; a script-spawned second px4 would
# fight it over the model and the MAVLink ports.
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SUPERVISOR_PID_FILE="${REPO_ROOT}/target/xtask-sim/supervisor.pid"
if [[ -f "${SUPERVISOR_PID_FILE}" ]] && kill -0 "$(cat "${SUPERVISOR_PID_FILE}")" 2>/dev/null; then
  echo "done — the xtask supervisor restarts PX4; re-arm from the browser once it logs ready"
  exit 0
fi

ROOTFS="${PX4_DIR}/build/px4_sitl_default/rootfs"
mkdir -p "${ROOTFS}"
cd "${ROOTFS}"
PX4_GZ_STANDALONE=1 PX4_SYS_AUTOSTART=4001 PX4_SIM_MODEL=gz_x500 \
  PX4_GZ_MODEL_NAME=x500_0 PX4_GZ_WORLD="${WORLD}" \
  nohup ../bin/px4 ../etc -s etc/init.d-posix/rcS -d > /tmp/px4_manual.log 2>&1 &
echo "done — re-arm from the browser once PX4 logs ready (~10 s)"
