#!/usr/bin/env bash
# Verify that one datum vocabulary reaches every reader of a geodetic fix.
#
# The wire carries a datum as a number. The Rust producer, the proto comment
# a reader implements from, and the browser decoder each hold their own copy
# of what those numbers mean. A variant added to one copy and not another is
# a position read against the wrong reference, which is the failure ADR-0022
# exists to prevent, so the copies are compared here.
set -euo pipefail

root="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
datum="$root/crates/pilotage-geo/src/datum.rs"
proto="$root/schemas/pilotage/v1/telemetry.proto"
decoder="$root/clients/web/wire.js"
# The browser reads telemetry through the wasm decoder, not through
# wire.js. A field only the JavaScript decoder knows is a field the
# client never sees, and no decode error says so.
wasm_decoder="$root/clients/web-instruments/src/decode_envelope/groups.rs"
mapper="$root/hosts/session-host/src/runtime/engine_actor/telemetry.rs"
status=0

for path in "$datum" "$proto" "$decoder" "$mapper"; do
    if [ ! -f "$path" ]; then
        echo "FORBIDDEN: required geodetic file is missing: $path" >&2
        exit 1
    fi
done

# Highest code a `from_u8` decoder accepts, which is the crate's own
# statement of how many variants can reach the wire.
highest_code() {
    awk -v enum="$1" '
        $0 ~ "pub enum " enum " \\{" { inside = 1 }
        inside && /=> Some\(Self::/ { if ($1 + 0 > top) top = $1 + 0 }
        inside && /_ => None/ { inside = 0 }
        END { print top + 0 }
    ' "$datum"
}

# The proto comment is the contract a reader implements from, so it must name
# every code the crate can produce, and the browser must refuse anything past
# the last one.
check_enum() {
    enum="$1"
    field="$2"
    decoder_name="$3"
    top="$(highest_code "$enum")"
    if [ "$top" -lt 1 ]; then
        echo "FORBIDDEN: $enum declares no wire codes" >&2
        status=1
        return
    fi
    comment="$(grep -A 8 "pilotage_geo::$enum" "$proto" | head -8)"
    code=1
    while [ "$code" -le "$top" ]; do
        if ! printf '%s' "$comment" | grep -Eq "(^|[^0-9])$code [a-zA-Z]"; then
            echo "FORBIDDEN: telemetry.proto does not name $enum code $code" >&2
            status=1
        fi
        code=$((code + 1))
    done
    if ! grep -Fq "$decoder_name > $top" "$decoder"; then
        echo "FORBIDDEN: the browser must refuse a $enum code past $top" >&2
        status=1
    fi
}

check_enum HorizontalDatum horizontal_datum horizontalDatum
check_enum VerticalDatum vertical_datum verticalDatum

# The producer converts through the typed codec, so a discriminant that
# moves cannot be silently re-numbered by an `as` cast.
if grep -Eq 'horizontal_datum as u32|vertical.datum as u32|\.datum as u32' "$mapper"; then
    echo "FORBIDDEN: a datum must reach the wire through to_u8(), not a cast" >&2
    status=1
fi
if ! grep -Fq 'position.horizontal_datum.to_u8()' "$mapper" \
    || ! grep -Fq 'vertical.datum.to_u8()' "$mapper"; then
    echo "FORBIDDEN: the wire mapping must convert datums through to_u8()" >&2
    status=1
fi

# An absent fix must stay absent. A default anywhere on this path would be
# Null Island, a real place, drawn as a plausible vehicle. The check is on
# what a default needs rather than on the shape of one expression, because
# the formatter wraps a long expression across lines and a line-local
# pattern would stop matching the very form the formatter produces.
if grep -Fq 'GeodeticFix::default' "$mapper"; then
    echo "FORBIDDEN: an absent geodetic fix must not take a default" >&2
    status=1
fi
# Both lanes reach the wire through the one mapping that re-runs the
# contract, and each gates on its own role first. Two call sites, so the
# count is what is checked: a lane that stopped calling it would still
# match a bare name.
if [ "$(grep -c 'and_then(geodetic_to_wire)' "$mapper")" != "2" ]; then
    echo "FORBIDDEN: each lane must reach the wire through the mapping, and only through it" >&2
    status=1
fi

# The mapping is the last place before the wire, and the typed value has
# public fields, so a producer can assemble one the constructor would have
# refused. The mapping re-runs the contract.
# `validate` discards the value the constructor would have produced, and
# the constructor is what wraps a longitude into range. The mapping has to
# send that value, or the readers disagree about the same bytes: one wraps
# 200 degrees east to 160 west and draws a vehicle, the other refuses the
# fix and draws none.
if ! grep -Fq 'position.normalized()' "$mapper"; then
    echo "FORBIDDEN: the wire mapping must send the normalized position" >&2
    status=1
fi

# The simulator's declared separation names no geoid; an operational role
# carrying it would read as a surveyed height from a model nothing names.
if ! grep -Fq 'SIMULATOR_GEOID_MODEL_ID' "$mapper" \
    || ! grep -Fq 'SourceRole::SimulationTruth' "$mapper"; then
    echo "FORBIDDEN: the simulator separation must be refused outside the truth role" >&2
    status=1
fi

# Each lane gates its fix on its own role: a position is where a
# mislabelled lane actually draws. Both decoders gate, and the producer
# gates too, on the side that can name the offending source.
if ! grep -Fq 'geodeticStamp.role !== 1' "$decoder"; then
    echo "FORBIDDEN: the estimate lane must gate its fix on the estimate role" >&2
    status=1
fi
if ! grep -Fq 'SourceRole::OperationalEstimate' "$wasm_decoder"; then
    echo "FORBIDDEN: the decoder the client runs must gate the estimate lane" >&2
    status=1
fi
if ! grep -Fq 'fix.stamp.role == SourceRole::OperationalEstimate' "$mapper"; then
    echo "FORBIDDEN: the estimate lane must refuse another role at the producer" >&2
    status=1
fi
if ! grep -Fq 'fix.stamp.role == SourceRole::SimulationTruth' "$mapper"; then
    echo "FORBIDDEN: the truth lane must refuse another role at the producer" >&2
    status=1
fi

# Every lane the wire carries a fix on must be surfaced by the decoder the
# client runs. A lane only wire.js knows reaches no reader.
for lane in geodetic geodetic_stamp; do
    if ! grep -Fq "$lane," "$wasm_decoder"; then
        echo "FORBIDDEN: the decoder the client runs must surface $lane" >&2
        status=1
    fi
done
if ! grep -Fq 'fix.longitude_deg < 180.0' "$wasm_decoder"; then
    echo "FORBIDDEN: the decoder the client runs must refuse an unnormalized longitude" >&2
    status=1
fi

# An origin is a u64. Read as a Number it collapses past 2^53 and two
# different origins compare equal, which is what the identity exists to
# prevent.
if ! grep -Fq 'firstBigVarint(f, 10)' "$decoder"; then
    echo "FORBIDDEN: the local origin must decode through the 64-bit path" >&2
    status=1
fi

# The guardrails only hold while CI runs them.
ci="$root/.github/workflows/ci.yml"
for step in 'clients/web/geodetic-fix.test.mjs' 'scripts/check-geodetic-datum-codes.sh'; do
    if ! grep -E '^[[:space:]]*run:' "$ci" | grep -Fq "$step"; then
        echo "FORBIDDEN: CI must run $step" >&2
        status=1
    fi
done

if [ "$status" -ne 0 ]; then
    echo "Geodetic datum codes: FAILED" >&2
    exit 1
fi

echo "Geodetic datum codes: OK"
