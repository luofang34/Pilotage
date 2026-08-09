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
  sceneDigestHex compositionDigestHex glyphAsset InstrumentBridge compositionFrame; do
  grep -q "$symbol" "$output_root"/*.swift
done

if [ "$(uname -s)" = "Darwin" ]; then
  generated_swift="$(find "$output_root" -maxdepth 1 -name '*.swift' -print -quit)"
  module_map="$output_root/pilotage_instrument_apple_bridgeFFI.modulemap"
  swiftc -parse-as-library -I "$output_root" \
    -Xcc "-fmodule-map-file=$module_map" "$generated_swift" \
    -emit-module -o "$output_root/pilotage_instrument_apple_bridge.swiftmodule"
  swiftc -parse-as-library -I "$output_root" \
    -Xcc "-fmodule-map-file=$module_map" "$generated_swift" \
    -emit-object -o "$output_root/pilotage_instrument_apple_bridge.o"
  swift test --package-path clients/apple-instrument-consumer
  indicate_source_root="$(
    find clients/apple-instrument-consumer/.build/checkouts \
      -type d -path '*/Sources/IndicateAppleDisplay' -print -quit
  )"
  if [ -z "$indicate_source_root" ]; then
    echo "IndicateAppleDisplay source checkout is missing" >&2
    exit 1
  fi
  swiftc -parse-as-library \
    "$indicate_source_root"/*.swift \
    -module-name IndicateAppleDisplay \
    -emit-module \
    -emit-module-path "$output_root/IndicateAppleDisplay.swiftmodule" \
    -emit-library -static \
    -o "$output_root/libIndicateAppleDisplay.a"
  swiftc -parse-as-library \
    -I "$output_root" \
    clients/apple-instrument-consumer/Sources/PilotageAppleInstrumentConsumer/*.swift \
    -module-name PilotageAppleInstrumentConsumer \
    -emit-module \
    -emit-module-path "$output_root/PilotageAppleInstrumentConsumer.swiftmodule" \
    -emit-library -static \
    -o "$output_root/libPilotageAppleInstrumentConsumer.a"
  swiftc -parse-as-library \
    -I "$output_root" \
    -Xcc "-fmodule-map-file=$module_map" \
    clients/apple-instrument-bridge/swift/GeneratedBridgeAdapter.swift \
    -emit-module -o "$output_root/PilotageGeneratedBridgeAdapter.swiftmodule"
  swiftc -parse-as-library \
    -I "$output_root" \
    -Xcc "-fmodule-map-file=$module_map" \
    clients/apple-instrument-bridge/swift/GeneratedBridgeAdapter.swift \
    clients/apple-instrument-bridge/swift/GeneratedBridgeIntegration.swift \
    "$output_root/pilotage_instrument_apple_bridge.o" \
    "$output_root/libPilotageAppleInstrumentConsumer.a" \
    "$output_root/libIndicateAppleDisplay.a" \
    "$library" \
    -o "$output_root/generated-bridge-integration"
  DYLD_LIBRARY_PATH="$(dirname "$library")" \
    "$output_root/generated-bridge-integration"
  (
    cd clients/apple-instrument-consumer
    xcodebuild -scheme PilotageAppleInstrumentConsumer \
      -destination 'generic/platform=iOS Simulator' \
      -derivedDataPath "$output_root/DerivedData" \
      CODE_SIGNING_ALLOWED=NO build
  )
fi

echo "Apple instrument consumer checks passed"
