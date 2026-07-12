#!/usr/bin/env bash
# Builds the instrument-panel WASM resource and generated browser binding.
# Both outputs are gitignored and must be rebuilt before browser tests/serving.
set -euo pipefail
cd "$(dirname "$0")/.."

required_bindgen="wasm-bindgen 0.2.126"
actual_bindgen="$(wasm-bindgen --version 2>/dev/null || true)"
if [ "$actual_bindgen" != "$required_bindgen" ]; then
  echo "build-web-instruments: required '$required_bindgen', found '${actual_bindgen:-not installed}'" >&2
  echo "install with: cargo install wasm-bindgen-cli --version 0.2.126 --locked" >&2
  exit 1
fi

rustup target list --installed | grep -q '^wasm32-unknown-unknown$' ||
  rustup target add wasm32-unknown-unknown

cargo build -p pilotage-instruments-web --target wasm32-unknown-unknown --release
wasm-bindgen \
  target/wasm32-unknown-unknown/release/pilotage_instruments_web.wasm \
  --target web \
  --out-dir clients/web \
  --out-name instrument-runtime \
  --no-typescript

wasm_bytes="$(wc -c < clients/web/instrument-runtime_bg.wasm | tr -d ' ')"
echo "built clients/web/instrument-runtime.js and instrument-runtime_bg.wasm (${wasm_bytes} bytes)"
