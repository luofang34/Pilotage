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

# Where the vehicle parks between attempts. Captured once per X-Plane
# boot (the launcher deletes the file when it starts a fresh X-Plane):
# reset_flight reloads the flight WHERE THE VEHICLE IS, so a reset
# after a flyaway would otherwise restart the session in a field.
HOME_FILE="${HOME}/.pilotage/xplane-home.json"
python3 - "$HOME_FILE" <<'PY_HOME'
import json, os, socket, struct, sys, time
path = sys.argv[1]
sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
sock.bind(("0.0.0.0", 0)); sock.settimeout(0.5)
# onground_any guards the capture: a reset can run while the vehicle
# is airborne, and a home captured mid-air makes every later reset
# spawn the vehicle at altitude and drop it.
REFS = [(1, "sim/flightmodel/position/local_x"),
        (2, "sim/flightmodel/position/local_y"),
        (3, "sim/flightmodel/position/local_z"),
        (4, "sim/flightmodel/failures/onground_any")]
def read_all():
    for idx, ref in REFS:
        sock.sendto(struct.pack("<4sxii400s", b"RREF", 5, idx, ref.encode()), ("127.0.0.1", 49000))
    got, end = {}, time.time() + 2
    while time.time() < end and len(got) < len(REFS):
        try: data, _ = sock.recvfrom(4096)
        except socket.timeout: continue
        if not data.startswith(b"RREF"): continue
        body = data[5:]
        for off in range(0, len(body) - 7, 8):
            idx, val = struct.unpack_from("<if", body, off)
            got[idx] = val
    for idx, ref in REFS:
        sock.sendto(struct.pack("<4sxii400s", b"RREF", 0, idx, ref.encode()), ("127.0.0.1", 49000))
    return got
if not os.path.exists(path):
    got = read_all()
    if len(got) == len(REFS) and got[4] >= 0.5:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        json.dump({"x": got[1], "y": got[2], "z": got[3]}, open(path, "w"))
        print(f"saved home position to {path}")
    elif len(got) == len(REFS):
        print("vehicle is airborne; home capture deferred to a grounded reset")
PY_HOME

echo "reloading the X-Plane flight..."
send_cmnd "sim/operation/reset_flight"

echo "restarting the flight controller..."
# Match ONLY this checkout's SITL binaries: bare patterns would kill
# unrelated sessions on the machine. The AVIATE binary must restart on
# reset too — its telemetry clock anchors at process boot, and the
# adapter's reset latch clears only when a FRESH boot clock opens a new
# source epoch.
PX4_DIR="${PX4_DIR:-$HOME/PX4-Autopilot}"
pkill -9 -f "${PX4_DIR}/build/px4_sitl_default/bin/px4" 2>/dev/null || true
pkill -TERM -f "Aviate/target/debug/sitl-xplane-alia250" 2>/dev/null || true

# The bridge needs a moment to observe the rewind and disconnect before
# its listener can arm again; the reloaded flight needs one to settle
# before a teleport sticks.
sleep 3

# Return the vehicle to its parking spot with its velocity zeroed, so a
# reset after a flyaway is a reset, not a relocation.
python3 - "$HOME_FILE" <<'PY_TELEPORT'
import json, os, socket, struct, sys
path = sys.argv[1]
if os.path.exists(path):
    home = json.load(open(path))
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    def write(ref, value):
        req = bytearray(b"DREF\x00") + struct.pack("<f", value)
        name = ref.encode().ljust(500, b" ")
        name = name[: len(ref)] + b"\x00" + name[len(ref) + 1 :]
        sock.sendto(bytes(req) + name, ("127.0.0.1", 49000))
    # Daylight: the demo camera is worthless over a midnight
    # airfield, and X-Plane follows the wall clock unless told
    # otherwise. Zulu is the writable dataref; local_time_sec is
    # derived from it.
    write("sim/time/use_system_time", 0.0)
    write("sim/time/zulu_time_sec", 61200.0)
    for axis in ("local_vx", "local_vy", "local_vz"):
        write(f"sim/flightmodel/position/{axis}", 0.0)
    write("sim/flightmodel/position/local_x", home["x"])
    write("sim/flightmodel/position/local_y", home["y"])
    write("sim/flightmodel/position/local_z", home["z"])
    print("vehicle returned to its parking spot")
PY_TELEPORT

echo "re-arming the SITL listener..."
send_cmnd "px4xplane/connect"

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
