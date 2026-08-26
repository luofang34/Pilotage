#!/usr/bin/env bash
# Self-test: the flight-build gate refuses each simulation-only crate.
#
# A gate that names crates in a pattern is only as good as the pattern. This
# builds the same dependency tree the gate reads, then asks the gate's own
# refusal to accept a tree carrying each forbidden crate in turn. A crate
# dropped from the pattern is a crate the flight build could carry.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${REPO_ROOT}"

# The pattern the gate applies, read from the gate rather than restated, so
# the two cannot drift apart.
forbidden="$(sed -n "s/^FORBIDDEN='\(.*\)'$/\1/p" scripts/check-flight-build.sh)"
if [ -z "${forbidden}" ]; then
    echo "test-check-flight-build: the gate states no forbidden pattern" >&2
    exit 1
fi

# Every simulation-only crate in the workspace must appear in the pattern.
# The list is the crates that exist to drive, watch or tune a simulator.
required=(
    pilotage-adapter-gazebo
    pilotage-sim-video
    pilotage-adapter-reference
    aviate-xil-contract
    aviate-xil-shm
    pilotage-tuning-feedback
    flight-tune
)
status=0
for crate in "${required[@]}"; do
    if ! grep -qE "(^|\|)${crate}(\||$)" <<<"${forbidden}"; then
        echo "test-check-flight-build: '${crate}' is not in the gate's pattern" >&2
        status=1
    fi
done

# The pattern must actually reject a tree that carries one. A pattern that
# matched nothing would pass the check above and gate nothing.
for crate in "${required[@]}"; do
    if ! grep -qE "${forbidden}" <<<"fake-tree ${crate} v0.1.0"; then
        echo "test-check-flight-build: the pattern does not match '${crate}'" >&2
        status=1
    fi
done

# And it must not reject the flight host itself, which would make the gate
# fail for every build rather than for a simulation-only one.
if grep -qE "${forbidden}" <<<"pilotage-session-host v0.1.0"; then
    echo "test-check-flight-build: the pattern rejects the flight host" >&2
    status=1
fi

if [ "${status}" -ne 0 ]; then
    exit 1
fi
echo "flight build gate names every simulation-only crate"
