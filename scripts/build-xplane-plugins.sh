#!/usr/bin/env bash
# Builds and installs the two X-Plane plugins the px4-xplane backend
# needs, plus the packaged QuadTailsitter aircraft:
#
#   1. px4xplane      - the MAVLink HIL bridge (external checkout,
#                       PX4XPLANE_DIR, default ../px4xplane)
#   2. PilotageAutoFlight - this repository's unattended flight starter
#                       (sim/xplane/autoflight)
#   3. PilotageCamera  - this repository's vehicle camera export
#                       (sim/xplane/camera)
#
# The install target is the X-Plane root: XPLANE_ROOT, else the first
# entry of the official installer registry that holds X-Plane.app.
# macOS only: the flight build never runs this script.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "build-xplane-plugins: macOS only (X-Plane target platform)" >&2
  exit 1
fi

# --- Resolve directories -------------------------------------------------
PX4XPLANE_DIR="${PX4XPLANE_DIR:-${REPO_ROOT}/../px4xplane}"
if [[ ! -f "${PX4XPLANE_DIR}/CMakeLists.txt" ]]; then
  echo "px4xplane checkout not found at ${PX4XPLANE_DIR}" >&2
  echo "clone https://github.com/alireza787b/px4xplane next to this repo or set PX4XPLANE_DIR" >&2
  exit 1
fi

if [[ -z "${XPLANE_ROOT:-}" ]]; then
  REGISTRY="${HOME}/Library/Preferences/x-plane_install_12.txt"
  while IFS= read -r line; do
    line="${line%"${line##*[![:space:]]}"}"
    if [[ -n "${line}" && -d "${line}/X-Plane.app" ]]; then
      XPLANE_ROOT="${line}"
      break
    fi
  done < "${REGISTRY}"
fi
if [[ -z "${XPLANE_ROOT:-}" || ! -d "${XPLANE_ROOT}/X-Plane.app" ]]; then
  echo "X-Plane 12 installation not found; set XPLANE_ROOT" >&2
  exit 1
fi
echo "X-Plane root: ${XPLANE_ROOT}"

# --- Build px4xplane -----------------------------------------------------
echo "building px4xplane..."
git -C "${PX4XPLANE_DIR}" submodule update --init --recursive
cmake -S "${PX4XPLANE_DIR}" -B "${PX4XPLANE_DIR}/build" -DCMAKE_BUILD_TYPE=Release >/dev/null
cmake --build "${PX4XPLANE_DIR}/build" -j "$(sysctl -n hw.ncpu)" >/dev/null
PACKAGE="${PX4XPLANE_DIR}/build/mac/Release/px4xplane"
[[ -f "${PACKAGE}/64/mac.xpl" ]] || { echo "px4xplane build produced no mac.xpl" >&2; exit 1; }

# --- Build PilotageAutoFlight -------------------------------------------
echo "building PilotageAutoFlight..."
AUTOFLIGHT_BUILD="${REPO_ROOT}/target/xplane-autoflight"
mkdir -p "${AUTOFLIGHT_BUILD}"
clang++ -std=c++17 -O2 -shared -fPIC \
  -DAPL=1 -DIBM=0 -DLIN=0 -DXPLM200 -DXPLM210 -DXPLM300 -DXPLM301 -DXPLM303 \
  -I "${PX4XPLANE_DIR}/lib/SDK/CHeaders/XPLM" \
  "${REPO_ROOT}/sim/xplane/autoflight/PilotageAutoFlight.cpp" \
  -undefined dynamic_lookup \
  -o "${AUTOFLIGHT_BUILD}/mac.xpl"

# --- Build PilotageCamera ------------------------------------------------
echo "building PilotageCamera..."
CAMERA_BUILD="${REPO_ROOT}/target/xplane-camera"
CAMERA_SRC="${REPO_ROOT}/sim/xplane/camera"
mkdir -p "${CAMERA_BUILD}"
clang++ -std=c++17 -O2 -shared -fPIC \
  -DAPL=1 -DIBM=0 -DLIN=0 -DXPLM200 -DXPLM210 -DXPLM300 -DXPLM301 -DXPLM303 \
  -DGL_SILENCE_DEPRECATION \
  -I "${PX4XPLANE_DIR}/lib/SDK/CHeaders/XPLM" -I "${CAMERA_SRC}" \
  "${CAMERA_SRC}/PilotageCamera.cpp" \
  "${CAMERA_SRC}/camera_state.cpp" \
  "${CAMERA_SRC}/view.cpp" \
  "${CAMERA_SRC}/link.cpp" \
  "${CAMERA_SRC}/capture.cpp" \
  "${CAMERA_SRC}/hud.cpp" \
  -undefined dynamic_lookup -framework OpenGL \
  -o "${CAMERA_BUILD}/mac.xpl"

# --- Install -------------------------------------------------------------
echo "installing plugins and aircraft..."
PLUGINS="${XPLANE_ROOT}/Resources/plugins"

# Keep the operator's active airframe selection across reinstalls.
ACTIVE_CONFIG=""
if [[ -f "${PLUGINS}/px4xplane/64/config.ini" ]]; then
  ACTIVE_CONFIG="$(grep -E '^config_name' "${PLUGINS}/px4xplane/64/config.ini" || true)"
fi
rm -rf "${PLUGINS}/px4xplane"
cp -R "${PACKAGE}" "${PLUGINS}/px4xplane"
if [[ -n "${ACTIVE_CONFIG}" ]]; then
  sed -i '' "s/^config_name.*/${ACTIVE_CONFIG}/" "${PLUGINS}/px4xplane/64/config.ini"
fi
# A desktop X-Plane stalls its run loop on texture loads and UI work.
# The bridge's 500 ms actuator-feedback deadline reads such a stall as a
# dead FC and disconnects, which wedges PX4's frozen lockstep clock.
# Widen both deadlines for this interactive-desktop deployment.
#
# The plugin resolves its config path from XPLMGetPluginInfo; when that
# yields a bare file name the plugin falls back to the process working
# directory, which is the X-Plane root. Keep a synchronized copy there
# so both resolution outcomes read the same configuration.
cp "${PLUGINS}/px4xplane/64/config.ini" "${XPLANE_ROOT}/config.ini"
for CONFIG in "${PLUGINS}/px4xplane/64/config.ini" "${XPLANE_ROOT}/config.ini"; do
  if [[ -n "${ACTIVE_CONFIG}" ]]; then
    sed -i '' "s/^config_name.*/${ACTIVE_CONFIG}/" "${CONFIG}"
  fi
  sed -i '' \
    -e 's/^hil_sensor_feedback_timeout_ms.*/hil_sensor_feedback_timeout_ms = 5000/' \
    -e 's/^hil_sensor_feedback_startup_timeout_ms.*/hil_sensor_feedback_startup_timeout_ms = 60000/' \
    "${CONFIG}"
done

mkdir -p "${PLUGINS}/PilotageAutoFlight/64"
cp "${AUTOFLIGHT_BUILD}/mac.xpl" "${PLUGINS}/PilotageAutoFlight/64/mac.xpl"

mkdir -p "${PLUGINS}/PilotageCamera/64"
cp "${CAMERA_BUILD}/mac.xpl" "${PLUGINS}/PilotageCamera/64/mac.xpl"

mkdir -p "${XPLANE_ROOT}/Aircraft/Extra Aircraft"
rm -rf "${XPLANE_ROOT}/Aircraft/Extra Aircraft/QuadTailsitter"
cp -R "${PX4XPLANE_DIR}/aircraft/QuadTailsitter" "${XPLANE_ROOT}/Aircraft/Extra Aircraft/"
# The aircraft source keeps its airfoils in "Airfoil/", but X-Plane 12
# resolves them from "<aircraft>/airfoils/"; without this copy the load
# stops on a modal missing-airfoil alert.
if [[ -d "${XPLANE_ROOT}/Aircraft/Extra Aircraft/QuadTailsitter/Airfoil" ]]; then
  mkdir -p "${XPLANE_ROOT}/Aircraft/Extra Aircraft/QuadTailsitter/airfoils"
  cp "${XPLANE_ROOT}/Aircraft/Extra Aircraft/QuadTailsitter/Airfoil/"*.afl \
    "${XPLANE_ROOT}/Aircraft/Extra Aircraft/QuadTailsitter/airfoils/"
fi

echo "done: px4xplane + PilotageAutoFlight + PilotageCamera + QuadTailsitter installed"
