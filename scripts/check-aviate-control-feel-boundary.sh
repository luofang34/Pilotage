#!/usr/bin/env bash
# Keep Aviate flight-response constants in the typed control-feel artifact.
set -euo pipefail

root_dir="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
declaration_pattern='^[[:space:]]*(pub(\([^)]*\))?[[:space:]]+)?(const|static)[[:space:]]+[A-Z][A-Z0-9_]*'
response_name='(^|_)(HORIZONTAL|VERTICAL|YAW|TILT|THRUST|THROTTLE|TAKEOFF|ACCEL|JERK|DEADZONE|EXPO|HOLD|RATE|ROLL|PITCH|SPEED|RESPONSE|CURVE|NEUTRAL|DWELL|RELEASE|REVERSAL|RAMP|SLEW|QUIET|LIMIT|APPLY|SETTLE|DT|TIME|TIMEOUT|TICKS|STEPS|PERIOD|DELAY|THRESHOLD)($|_)'
response_type='(^|[^[:alnum:]_])(f32|f64|Duration)([^[:alnum:]_]|$)'
exemptions_seen=$'\n'

if grep -Eq \
    '^[[:space:]]*pub[[:space:]]+((async|const)[[:space:]]+)*fn[[:space:]]+(new_with_profile|install_profile)[[:space:]]*\(' \
    "$root_dir/adapters/aviate/src/uplink.rs"; then
    echo "adapters/aviate/src/uplink.rs:custom-profile-entry" >&2
    echo "FORBIDDEN: a public profile mutation can bypass the execution target" >&2
    exit 1
fi

is_allowed() {
    local key="$1:$2"
    case "$key" in
        'adapters/aviate/src/link/measurement.rs:RESET_RECEIVE_DWELL' \
        | 'adapters/aviate/src/link/measurement.rs:RESET_CANDIDATE_MAX_MS' \
        | 'adapters/aviate/src/link/measurement.rs:RESET_SOURCE_DWELL_MS' \
        | 'adapters/aviate/src/link/measurement.rs:RESET_SILENCE' \
        | 'adapters/aviate/src/uplink.rs:MAX_DT_S' \
        | 'adapters/aviate/src/uplink.rs:ARM_QUIET' \
        | 'adapters/aviate/src/adapter/control.rs:RESET_CLEAR_DEADBAND' \
        | 'adapters/aviate/src/adapter.rs:GIMBAL_NEUTRAL_BUTTON' \
        | 'adapters/aviate/src/adapter.rs:ROLL_AXIS' \
        | 'adapters/aviate/src/adapter.rs:PITCH_AXIS' \
        | 'adapters/aviate/src/adapter.rs:THROTTLE_AXIS' \
        | 'adapters/aviate/src/adapter.rs:YAW_AXIS' \
        | 'adapters/aviate/src/adapter.rs:WITHHOLD_AFTER' \
        | 'adapters/aviate/src/adapter/shm_sampling.rs:REATTACH_INTERVAL' \
        | 'adapters/aviate/src/adapter/pointing.rs:PAN_LIMIT_RAD' \
        | 'adapters/aviate/src/adapter/pointing.rs:TILT_MIN_RAD' \
        | 'adapters/aviate/src/adapter/pointing.rs:TILT_MAX_RAD' \
        | 'adapters/aviate/src/adapter/pointing.rs:MAX_PITCH_RATE_RPS' \
        | 'adapters/aviate/src/adapter/pointing.rs:MAX_YAW_RATE_RPS' \
        | 'adapters/aviate/src/adapter/pointing.rs:INTEGRATION_STEP_S' \
        | 'adapters/aviate/src/adapter/pointing.rs:PAYLOAD_VIEW_HOLD')
            if [[ "$exemptions_seen" == *$'\n'"$key"$'\n'* ]]; then
                return 1
            fi
            exemptions_seen+="$key"$'\n'
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

set +e
matches="$(cd "$root_dir" && grep -RInE \
    --include='*.rs' \
    --exclude='tests.rs' \
    --exclude-dir='tests' \
    "$declaration_pattern" \
    'adapters/aviate/src')"
status=$?
set -e
if [[ $status -gt 1 ]]; then
    exit "$status"
fi

violations=""
while IFS= read -r match; do
    [[ -z "$match" ]] && continue
    if [[ ! "$match" =~ ^([^:]+):[0-9]+:(.*)$ ]]; then
        echo "the Aviate control-feel guard could not parse a declaration" >&2
        exit 1
    fi
    path="${BASH_REMATCH[1]}"
    declaration="${BASH_REMATCH[2]}"
    if [[ ! "$declaration" =~ (const|static)[[:space:]]+([A-Z][A-Z0-9_]*) ]]; then
        echo "the Aviate control-feel guard could not parse an identifier" >&2
        exit 1
    fi
    identifier="${BASH_REMATCH[2]}"
    type_text="${declaration#*:}"
    type_text="${type_text%%=*}"
    if [[ ! "$identifier" =~ $response_name ]] \
        && [[ ! "$identifier" =~ _MS$ ]] \
        && [[ ! "$type_text" =~ $response_type ]]; then
        continue
    fi
    if is_allowed "$path" "$identifier"; then
        continue
    fi
    violations+="$path:$identifier"$'\n'
done <<< "$matches"

if [[ -n "$violations" ]]; then
    printf '%s' "$violations"
    echo "FORBIDDEN: Aviate response constant bypasses the control-feel artifact" >&2
    exit 1
fi

echo "Aviate control-feel boundary: OK"
