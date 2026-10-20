#!/usr/bin/env bash
# Builds the JSBSim release the JSBSim sidecar links: clones it into
# JSBSIM_DIR (else ~/jsbsim) when absent, then configures, builds, and
# installs a static library into its install/ tree. It needs only git,
# CMake, and a C++ compiler: no Python module, no documentation.
#
# `cargo xtask build-jsbsim` runs this script and then validates the
# install. A checkout without the workspace's private dependencies (the
# sidecar builds on its own) runs it directly.
set -euo pipefail

# The release the sidecar is written against; xtask's JSBSIM_TAG names
# the same one, and its tests hold the two equal.
JSBSIM_TAG="v1.3.3"
JSBSIM_REPOSITORY="https://github.com/JSBSim-Team/jsbsim.git"

root="${JSBSIM_DIR:-}"
if [ -z "$root" ]; then
  if [ -z "${HOME:-}" ]; then
    echo "build-jsbsim: set JSBSIM_DIR to the JSBSim checkout (HOME is not set either)" >&2
    exit 1
  fi
  root="$HOME/jsbsim"
fi

if [ -f "$root/CMakeLists.txt" ] && [ -f "$root/src/FGFDMExec.h" ]; then
  echo "build-jsbsim: using the checkout at $root"
elif [ ! -e "$root" ] || [ -z "$(ls -A "$root")" ]; then
  echo "build-jsbsim: cloning JSBSim $JSBSIM_TAG into $root"
  git clone --depth 1 --branch "$JSBSIM_TAG" "$JSBSIM_REPOSITORY" "$root"
else
  echo "build-jsbsim: $root is not a JSBSim checkout; point JSBSIM_DIR at an empty or absent directory" >&2
  exit 1
fi

jobs="${JSBSIM_JOBS:-}"
if [ -z "$jobs" ]; then
  jobs="$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 2)"
fi

# Position-independent code, so a Linux link of the static library into
# a Rust binary does not need text relocations.
cmake -S "$root" -B "$root/build" \
  -DCMAKE_BUILD_TYPE=Release \
  -DBUILD_SHARED_LIBS=OFF \
  -DBUILD_PYTHON_MODULE=OFF \
  -DBUILD_DOCS=OFF \
  -DCMAKE_POSITION_INDEPENDENT_CODE=ON \
  "-DCMAKE_INSTALL_PREFIX=$root/install"
cmake --build "$root/build" --target install --parallel "$jobs"
echo "build-jsbsim: JSBSim $JSBSIM_TAG is installed in $root/install"
