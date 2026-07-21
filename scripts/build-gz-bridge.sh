#!/usr/bin/env bash
# Builds the C++ gz-transport camera sidecar (`pilotage-gz-bridge`) that the
# px4-gz backend uses to deliver Gazebo camera frames into the session host.
# The binary is gitignored; a fresh checkout must build it before the FPV/chase
# video works. The px4-gz launcher runs this automatically, best-effort — the
# sim degrades to no-video when the toolchain is absent.
set -euo pipefail
cd "$(dirname "$0")/.."

bridge_dir="adapters/gazebo/bridge"
build_dir="${bridge_dir}/build"

# gz-transport / gz-msgs come from Gazebo Harmonic; without them the sidecar
# cannot be built. Fail with the install path rather than a raw CMake error.
for pkg in gz-transport13 gz-msgs10; do
  if ! pkg-config --exists "$pkg"; then
    echo "build-gz-bridge: missing pkg-config module '$pkg'" >&2
    echo "install Gazebo Harmonic: brew install gz-harmonic (macOS) or see https://gazebosim.org/docs/harmonic/install" >&2
    exit 1
  fi
done

# gz-msgs links a specific protobuf; the generated C++ bindings MUST be produced
# by that same protoc or the ABI mismatches (PROTOBUF_CONSTEXPR / ClassData
# link errors). The PATH protoc is often an unrelated toolchain (e.g. a conda
# protoc), so prefer the protoc from the same brew formula gz-msgs links.
protoc_bin=""
if command -v brew >/dev/null 2>&1; then
  brew_protobuf="$(brew --prefix protobuf 2>/dev/null || true)"
  if [ -n "$brew_protobuf" ] && [ -x "$brew_protobuf/bin/protoc" ]; then
    protoc_bin="$brew_protobuf/bin/protoc"
  fi
fi
if [ -z "$protoc_bin" ]; then
  protoc_bin="$(command -v protoc || true)"
  if [ -z "$protoc_bin" ]; then
    echo "build-gz-bridge: no protoc found" >&2
    echo "install protobuf: brew install protobuf" >&2
    exit 1
  fi
  echo "build-gz-bridge: brew protobuf not found; using '$protoc_bin' (must match the protobuf gz-msgs links)" >&2
fi

echo "build-gz-bridge: configuring with protoc '$protoc_bin'"
cmake -S "$bridge_dir" -B "$build_dir" \
  -DCMAKE_BUILD_TYPE=Release \
  -DPROTOC_EXECUTABLE="$protoc_bin"
cmake --build "$build_dir" --parallel

binary="${build_dir}/pilotage-gz-bridge"
if [ ! -x "$binary" ]; then
  echo "build-gz-bridge: build finished but '$binary' is missing" >&2
  exit 1
fi
echo "built $binary"
