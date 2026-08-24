#!/usr/bin/env bash
# Test reset ordering and the fail-closed weather boundary.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
reset_script="${repo_root}/scripts/reset-xplane-sim.sh"
test_root="$(mktemp -d "${TMPDIR:-/tmp}/pilotage-reset-order.XXXXXX")"
trap 'rm -rf "${test_root}"' EXIT

run_reset_fixture() {
  local mode="$1"
  local event_log="$2"
  bash -s -- "${reset_script}" "${event_log}" "${mode}" <<'BASH'
source "$1"
event_log="$2"
mode="$3"

record() {
  printf '%s\n' "$1" >> "${event_log}"
}

capture_home() { record capture; }
stop_flight_controller() { record stop; }
reload_flight() { record reload; }
settle_reloaded_flight() { record settle; }
teleport_home() { record teleport; }
clear_xplane_weather() {
  record clear
  [[ "${mode}" == "success" ]]
}
rearm_bridge() { record rearm; }
restart_flight_controller() { record restart; }

reset_xplane_session
BASH
}

success_log="${test_root}/success.log"
run_reset_fixture success "${success_log}"
success_events="$(<"${success_log}")"
expected_success=$'capture\nstop\nreload\nsettle\nteleport\nclear\nrearm\nrestart'
if [[ "${success_events}" != "${expected_success}" ]]; then
  echo "reset success order is incorrect: ${success_events}" >&2
  exit 1
fi

failure_log="${test_root}/failure.log"
if run_reset_fixture failure "${failure_log}"; then
  echo "weather refusal did not fail the reset" >&2
  exit 1
fi
failure_events="$(<"${failure_log}")"
expected_failure=$'capture\nstop\nreload\nsettle\nteleport\nclear'
if [[ "${failure_events}" != "${expected_failure}" ]]; then
  echo "reset crossed the failed weather boundary: ${failure_events}" >&2
  exit 1
fi

echo "test-reset-xplane-order: OK"
