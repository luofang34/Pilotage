#!/usr/bin/env bash
# Builds the instrument-panel WASM module and places it where the web
# client loads it (clients/web/instruments.wasm, gitignored build output).
set -euo pipefail
cd "$(dirname "$0")/.."

rustup target list --installed | grep -q '^wasm32-unknown-unknown$' ||
  rustup target add wasm32-unknown-unknown

cargo build -p pilotage-instruments-web --target wasm32-unknown-unknown --release
cp target/wasm32-unknown-unknown/release/pilotage_instruments_web.wasm clients/web/instruments.wasm
echo "built clients/web/instruments.wasm ($(wc -c < clients/web/instruments.wasm | tr -d ' ') bytes)"
