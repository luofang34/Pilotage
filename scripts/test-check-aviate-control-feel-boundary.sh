#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bash "$repo_root/scripts/check-aviate-control-feel-boundary.sh" "$repo_root" >/dev/null
fixture="$(mktemp -d "${TMPDIR:-/tmp}/pilotage-control-feel-boundary.XXXXXX")"
trap 'rm -rf "$fixture"' EXIT
mkdir -p \
    "$fixture/adapters/aviate/src/adapter" \
    "$fixture/adapters/aviate/src/uplink"

printf '%s\n' 'const MAX_DT_S: f32 = 0.1;' > "$fixture/adapters/aviate/src/uplink.rs"
printf '%s\n' 'pub fn shape(value: f32) -> f32 { value }' \
    > "$fixture/adapters/aviate/src/uplink/feel.rs"
printf '%s\n' 'const RESET_CLEAR_DEADBAND: f32 = 0.05;' \
    > "$fixture/adapters/aviate/src/adapter/control.rs"
printf '%s\n' 'pub(crate) const MAX_YAW_RATE_RPS: f32 = 0.8;' \
    > "$fixture/adapters/aviate/src/adapter/pointing.rs"
bash "$repo_root/scripts/check-aviate-control-feel-boundary.sh" "$fixture" >/dev/null

assert_public_uplink_rejected() {
    local symbol="$1"
    local declaration="$2"
    local path="$fixture/adapters/aviate/src/uplink.rs"
    local output="$fixture/failure.txt"
    printf '%s\n' "$declaration" > "$path"
    if bash "$repo_root/scripts/check-aviate-control-feel-boundary.sh" "$fixture" \
        >"$output" 2>&1; then
        echo "the Aviate control-feel guard accepted public $symbol" >&2
        exit 1
    fi
    if ! grep -Fq 'custom-profile-entry' "$output"; then
        echo "the Aviate control-feel guard did not identify public $symbol" >&2
        exit 1
    fi
    printf '%s\n' 'const MAX_DT_S: f32 = 0.1;' > "$path"
}

assert_public_uplink_rejected \
    'new_with_profile' \
    'pub fn new_with_profile(profile: ValidatedFlightFeelProfile) {}'
assert_public_uplink_rejected \
    'install_profile' \
    'pub fn install_profile(profile: ValidatedFlightFeelProfile) {}'

assert_rejected() {
    local path="$1"
    local declaration="$2"
    local expected="$3"
    local output="$fixture/failure.txt"
    printf '%s\n' "$declaration" > "$fixture/$path"
    if bash "$repo_root/scripts/check-aviate-control-feel-boundary.sh" "$fixture" \
        >"$output" 2>&1; then
        echo "the Aviate control-feel guard accepted $expected" >&2
        exit 1
    fi
    if ! grep -Fq "$expected" "$output"; then
        echo "the Aviate control-feel guard did not identify $expected" >&2
        exit 1
    fi
    printf '%s\n' 'pub fn keep_fixture_valid() {}' > "$fixture/$path"
}

assert_rejected \
    'adapters/aviate/src/uplink/feel.rs' \
    'const MAX_TAKEOFF_THRUST: f32 = 0.75;' \
    'MAX_TAKEOFF_THRUST'
assert_rejected \
    'adapters/aviate/src/adapter/flight.rs' \
    'const MAX_ROLL_RATE: f32 = 0.6;' \
    'MAX_ROLL_RATE'
assert_rejected \
    'adapters/aviate/src/adapter/flight.rs' \
    'const RESPONSE_CURVE: [f32; 2] = [0.2, 0.8];' \
    'RESPONSE_CURVE'
assert_rejected \
    'adapters/aviate/src/adapter/flight.rs' \
    'const MAX_DT_S: f32 = 0.1;' \
    'MAX_DT_S'
assert_rejected \
    'adapters/aviate/src/adapter/flight.rs' \
    'static RELEASE_DWELL: std::time::Duration = std::time::Duration::from_millis(20);' \
    'RELEASE_DWELL'
assert_rejected \
    'adapters/aviate/src/adapter/flight.rs' \
    'const NEUTRAL_DWELL_MS: u32 = 20;' \
    'NEUTRAL_DWELL_MS'
assert_rejected \
    'adapters/aviate/src/adapter/flight.rs' \
    'const MAGIC: f32 = 0.4;' \
    'MAGIC'
assert_rejected \
    'adapters/aviate/src/adapter/flight.rs' \
    'const COMMAND_SETTLE: std::time::Duration = std::time::Duration::from_millis(20);' \
    'COMMAND_SETTLE'
assert_rejected \
    'adapters/aviate/src/adapter/flight.rs' \
    'const COMMAND_SETTLE_MS: u32 = 20;' \
    'COMMAND_SETTLE_MS'
assert_rejected \
    'adapters/aviate/src/adapter/flight.rs' \
    'const APPLY_STEPS: u32 = 4;' \
    'APPLY_STEPS'
assert_rejected \
    'adapters/aviate/src/adapter/flight.rs' \
    'const REVERSAL_STEPS: u32 = 4;' \
    'REVERSAL_STEPS'

printf '%s\n' \
    '// const RESPONSE_LIMIT: f32 = 0.4;' \
    'const MAGIC: DurationPolicy = DurationPolicy::Fixed;' \
    > "$fixture/adapters/aviate/src/adapter/flight.rs"
bash "$repo_root/scripts/check-aviate-control-feel-boundary.sh" "$fixture" >/dev/null

printf '%s\n' \
    'const MAX_DT_S: f32 = 0.1;' \
    'mod nested {' \
    '    const MAX_DT_S: f32 = 0.2;' \
    '}' \
    > "$fixture/adapters/aviate/src/uplink.rs"
if bash "$repo_root/scripts/check-aviate-control-feel-boundary.sh" "$fixture" \
    >"$fixture/failure.txt" 2>&1; then
    echo "the Aviate control-feel guard accepted a reused exemption" >&2
    exit 1
fi
if ! grep -Fq 'MAX_DT_S' "$fixture/failure.txt"; then
    echo "the Aviate control-feel guard did not identify a reused exemption" >&2
    exit 1
fi

echo "Aviate control-feel boundary self-test: OK"
