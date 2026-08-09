#!/usr/bin/env bash
# Generates the bridge binding and verifies the native Apple consumer.
set -euo pipefail
cd "$(dirname "$0")/.."

case "$(uname -s)" in
  Darwin) library="target/debug/libpilotage_instrument_apple_bridge.dylib" ;;
  Linux) library="target/debug/libpilotage_instrument_apple_bridge.so" ;;
  *) echo "unsupported host for Apple bridge generation" >&2; exit 1 ;;
esac

output_root="$(mktemp -d "${TMPDIR:-/tmp}/pilotage-apple-bindings.XXXXXX")"
trap 'rm -rf "$output_root"' EXIT

cargo build --locked -p pilotage-instrument-apple-bridge --lib
cargo run --locked --quiet -p pilotage-instrument-apple-bridge \
  --bin pilotage_uniffi_bindgen -- \
  generate --library --language swift --out-dir "$output_root" "$library"

for symbol in stateAbiVersion sceneFormatVersion corpusVersion corpusDigestHex \
  sceneDigestHex compositionDigestHex InstrumentBridge compositionFrame; do
  grep -q "$symbol" "$output_root"/*.swift
done

if [ "$(uname -s)" = "Darwin" ]; then
  generated_swift="$(find "$output_root" -maxdepth 1 -name '*.swift' -print -quit)"
  module_map="$output_root/pilotage_instrument_apple_bridgeFFI.modulemap"
  swiftc -parse-as-library -I "$output_root" \
    -Xcc "-fmodule-map-file=$module_map" "$generated_swift" \
    -emit-module -o "$output_root/pilotage_instrument_apple_bridge.swiftmodule"
  swift test --package-path clients/apple-instrument-consumer
  consumer_bin_path="$(
    swift build --package-path clients/apple-instrument-consumer --show-bin-path
  )"
  swiftc -parse-as-library \
    -I "$output_root" \
    -I "$consumer_bin_path" \
    -I "$consumer_bin_path/Modules" \
    -Xcc "-fmodule-map-file=$module_map" \
    clients/apple-instrument-bridge/swift/GeneratedBridgeAdapter.swift \
    -emit-module -o "$output_root/PilotageGeneratedBridgeAdapter.swiftmodule"
  (
    cd clients/apple-instrument-consumer
    xcodebuild -scheme PilotageAppleInstrumentConsumer \
      -destination 'generic/platform=iOS Simulator' \
      -derivedDataPath "$output_root/DerivedData" \
      CODE_SIGNING_ALLOWED=NO build
  )
fi

echo "Apple instrument consumer checks passed"
