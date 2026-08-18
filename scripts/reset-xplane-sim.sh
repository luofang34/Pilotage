#!/usr/bin/env bash
# Resets the X-Plane flight demo between test iterations: reloads the
# flight in X-Plane (the px4xplane bridge detects the simulation-time
# rewind and disconnects), re-arms the bridge's SITL listener, and
# restarts the px4 binary. Host and browser stay up - the MAVLink link
# detects the restarted stream as a new source epoch and the adapter's
# reset latch clears on fresh telemetry plus neutral input.
#
# Argument-free by contract: the host spawns PILOTAGE_RESET_CMD with no
# arguments.
set -euo pipefail

# X-Plane's UDP command port on the local machine.
XPLANE_UDP_PORT=49000

# Sends one X-Plane CMND datagram: "CMND\0" + command path + NUL.
send_cmnd() {
  python3 - "$1" <<'PY'
import socket, sys
payload = b"CMND\0" + sys.argv[1].encode() + b"\0"
socket.socket(socket.AF_INET, socket.SOCK_DGRAM).sendto(
    payload, ("127.0.0.1", 49000))
PY
}

echo "reloading the X-Plane flight..."
send_cmnd "sim/operation/reset_flight"

echo "restarting PX4..."
# Match ONLY this checkout's SITL binary: a bare "bin/px4" pattern
# would kill unrelated PX4 sessions on the machine.
PX4_DIR="${PX4_DIR:-$HOME/PX4-Autopilot}"
pkill -9 -f "${PX4_DIR}/build/px4_sitl_default/bin/px4" 2>/dev/null || true

# The bridge needs a moment to observe the rewind and disconnect before
# its listener can arm again.
sleep 3
echo "re-arming the SITL listener..."
send_cmnd "px4xplane/toggleEnable"

# When `cargo xtask sim` supervises the session, the supervisor restarts
# the flight-controller stage itself; a script-spawned second px4 would
# fight it over the MAVLink ports.
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SUPERVISOR_PID_FILE="${REPO_ROOT}/target/xtask-sim/supervisor.pid"
if [[ -f "${SUPERVISOR_PID_FILE}" ]] && kill -0 "$(cat "${SUPERVISOR_PID_FILE}")" 2>/dev/null; then
  echo "done - the xtask supervisor restarts PX4; re-arm from the browser once it logs ready"
  exit 0
fi

ROOTFS="${PX4_DIR}/build/px4_sitl_default/rootfs-xplane"
mkdir -p "${ROOTFS}"
cd "${ROOTFS}"
PX4_SIMULATOR=xplane PX4_SIM_HOSTNAME=127.0.0.1 \
  PX4_SYS_AUTOSTART="${PX4_SYS_AUTOSTART:-5021}" \
  nohup ../bin/px4 ../etc -s etc/init.d-posix/rcS -d > /tmp/px4_xplane_manual.log 2>&1 &
echo "done - re-arm from the browser once PX4 logs ready (~10 s)"
