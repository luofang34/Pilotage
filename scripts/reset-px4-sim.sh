#!/usr/bin/env bash
# Resets the PX4 flight demo from the viewer: the session host runs this
# script, and it starts the xtask reset for the host's own PX4 instance
# (PILOTAGE_PX4_INSTANCE, set by the launcher; 0 when absent).
#
# Usage: scripts/reset-px4-sim.sh
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${REPO_ROOT}"
exec cargo xtask reset --fc px4-gz --instance "${PILOTAGE_PX4_INSTANCE:-0}"
