#!/usr/bin/env bash
# Prove that the Aviate supervision guard rejects signal ownership changes.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
guard="$repo_root/scripts/check-aviate-supervision-boundary.sh"
fixture="$(mktemp -d "${TMPDIR:-/tmp}/pilotage-aviate-supervision-test.XXXXXX")"
trap 'rm -rf "$fixture"' EXIT

interface="$fixture/tools/flight-tune-aviate/src/supervisor/process_control.rs"
gate="$fixture/tools/flight-tune-aviate/src/supervisor/gate.rs"
owner_cleanup="$fixture/tools/flight-tune-aviate/src/supervisor/owner/cleanup.rs"
owner_launch="$fixture/tools/flight-tune-aviate/src/supervisor/owner/launch.rs"
recovery="$fixture/tools/flight-tune-aviate/src/process/recovery/cleanup.rs"
test_source="$fixture/tools/flight-tune-aviate/src/process/recovery/tests.rs"

mkdir -p \
    "$(dirname "$interface")" \
    "$(dirname "$owner_cleanup")" \
    "$(dirname "$recovery")"

write_clean_fixture() {
    printf '%s\n' \
        'fn stop_child(child: &mut std::process::Child) { child.kill(); }' \
        'fn signal_current_process_group() {' \
        '    rustix::process::kill_current_process_group(Signal::KILL);' \
        '}' \
        'fn signal_process_group(group: Pid) {' \
        '    rustix::process::kill_process_group(group, Signal::KILL);' \
        '}' \
        > "$interface"
    printf '%s\n' \
        'fn contain(child: &mut Child) { process_control::stop_child(child); }' \
        'fn fail() { process_control::signal_current_process_group(); }' \
        > "$gate"
    printf '%s\n' \
        'fn stop(group: u32) { process_control::signal_process_group(group); }' \
        > "$owner_cleanup"
    printf '%s\n' \
        'fn stop(child: &mut Child) { process_control::stop_child(child); }' \
        > "$owner_launch"
    printf '%s\n' 'fn recover() {}' > "$recovery"
    printf '%s\n' \
        'fn teardown(child: &mut Child) { child.kill(); }' \
        > "$test_source"
}

expect_failure() {
    local name="$1" expected="$2" output
    if output="$(bash "$guard" "$fixture" 2>&1)"; then
        echo "the Aviate supervision guard accepted $name" >&2
        exit 1
    fi
    if ! grep -Fq "$expected" <<<"$output"; then
        echo "the Aviate supervision guard gave the wrong result for $name" >&2
        echo "$output" >&2
        exit 1
    fi
}

write_clean_fixture
bash "$guard" "$fixture" >/dev/null

printf '%s\n' 'fn recover(child: &mut Child) { child.kill(); }' > "$recovery"
expect_failure \
    'a raw recovery signal' \
    'uses a raw process-control primitive'

write_clean_fixture
printf '%s\n' \
    'fn recover(group: u32) { process_control::signal_process_group(group); }' \
    > "$recovery"
expect_failure \
    'a recovery signal entry point' \
    'uses an unowned process-control entry point'

write_clean_fixture
printf '%s\n' 'fn stop(child: &mut Child) { child.kill(); }' > "$owner_cleanup"
expect_failure \
    'a raw owner signal' \
    'uses a raw process-control primitive'

write_clean_fixture
printf '%s\n' \
    'fn stop(group: u32) { process_control::signal_process_group(group); }' \
    > "$gate"
expect_failure \
    'a signal entry point with the wrong owner' \
    'uses an unowned process-control entry point'

write_clean_fixture
printf '%s\n' \
    'fn recover(pid: Pid) {' \
    '    rustix::process::kill_process(pid, Signal::KILL);' \
    '}' \
    > "$recovery"
expect_failure \
    'a direct process signal' \
    'uses a raw process-control primitive'

write_clean_fixture
printf '%s\n' \
    'fn recover(child: &mut Child) { std::process::Child::kill(child); }' \
    > "$recovery"
expect_failure \
    'a UFCS child signal' \
    'uses a raw process-control primitive'

write_clean_fixture
printf '%s\n' \
    'fn terminate_pid(pid: Pid) {' \
    '    rustix::process::kill_process(pid, Signal::KILL);' \
    '}' \
    >> "$interface"
printf '%s\n' \
    'fn recover(pid: Pid) { process_control::terminate_pid(pid); }' \
    > "$recovery"
expect_failure \
    'an unreviewed process-control wrapper' \
    'uses an unowned process-control entry point'

echo "check-aviate-supervision-boundary self-test: OK"
