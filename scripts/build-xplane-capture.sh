#!/usr/bin/env bash
# Builds the X-Plane window-capture video sidecar (ScreenCaptureKit,
# macOS only). Output: target/xplane-capture/pilotage-xplane-capture.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "build-xplane-capture: macOS only (ScreenCaptureKit)" >&2
  exit 1
fi

OUT_DIR="${REPO_ROOT}/target/xplane-capture"
mkdir -p "${OUT_DIR}"
swiftc -O \
  "${REPO_ROOT}/sim/xplane/capture/pilotage-xplane-capture.swift" \
  -o "${OUT_DIR}/pilotage-xplane-capture"
echo "built ${OUT_DIR}/pilotage-xplane-capture"
