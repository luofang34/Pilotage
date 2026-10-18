#!/usr/bin/env bash
# Keep the shared control runtime free of browser bindings, so native
# operator clients link the same mapping the browser runs instead of a
# copy. A wasm-bindgen pin in its graph would force every consumer onto
# Pilotage's exact wasm-bindgen version.
set -euo pipefail

root_dir="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
forbidden='^[[:space:]]*(wasm-bindgen|wasm-bindgen-futures|js-sys|web-sys|serde-wasm-bindgen|gloo[a-z0-9-]*)[[:space:]]*='

pending=(pilotage-control)
seen=$'\n'
status=0
while ((${#pending[@]} > 0)); do
    crate="${pending[0]}"
    pending=("${pending[@]:1}")
    [[ "$seen" == *$'\n'"$crate"$'\n'* ]] && continue
    seen+="$crate"$'\n'
    manifest="$root_dir/crates/$crate/Cargo.toml"
    if [[ ! -f "$manifest" ]]; then
        echo "crates/$crate/Cargo.toml: missing" >&2
        status=1
        continue
    fi
    if grep -nE "$forbidden" "$manifest" >&2; then
        echo "FORBIDDEN: crates/$crate reaches pilotage-control with a browser binding" >&2
        status=1
    fi
    # Follow workspace crates; their manifests are part of the runtime's graph.
    while read -r dependency; do
        pending+=("$dependency")
    done < <(grep -oE '^[[:space:]]*pilotage-[a-z0-9-]+[[:space:]]*=' "$manifest" \
        | sed -E 's/[[:space:]=]//g')
done
exit "$status"
