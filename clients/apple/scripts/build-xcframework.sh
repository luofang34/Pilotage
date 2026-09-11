#!/bin/sh
# Build the Rust slices, Swift bindings, and XCFramework.
set -eu

ios_deployment_target="${IPHONEOS_DEPLOYMENT_TARGET:-26.0}"
unset IPHONEOS_DEPLOYMENT_TARGET
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-15.0}"

client_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
ffi_root="$client_root/rust/pilotage-situation-ffi"
package_root="$client_root/Packages/PilotageCore"
cd "$ffi_root"

targets="aarch64-apple-ios aarch64-apple-ios-sim aarch64-apple-darwin"
for target in $targets; do
    if ! rustup target list --installed | grep -q "^$target$"; then
        rustup target add "$target"
    fi
    case "$target" in
        *-apple-ios*)
            IPHONEOS_DEPLOYMENT_TARGET="$ios_deployment_target" \
                cargo build --release --lib --target "$target"
            ;;
        *) cargo build --release --lib --target "$target" ;;
    esac
done

cargo build --release
cargo run --release --bin pilotage-situation-uniffi-bindgen -- generate \
    --library target/release/libpilotage_situation_ffi.dylib \
    --language swift --out-dir target/swift-bindings

generated="$package_root/Sources/PilotageCore/Generated"
headers=target/xcframework-headers
artifact="$package_root/artifacts/PilotageFFI.xcframework"
rm -rf "$generated" "$headers" "$artifact"
mkdir -p "$generated" "$headers" "$package_root/artifacts"
cp target/swift-bindings/pilotage_situation_ffi.swift "$generated/"
cp target/swift-bindings/pilotage_instrument_apple_bridge.swift "$generated/"
cp target/swift-bindings/pilotage_situation_ffiFFI.h "$headers/"
cp target/swift-bindings/pilotage_instrument_apple_bridgeFFI.h "$headers/"
# One static library carries both UniFFI namespaces; the module map
# declares both so either generated file can import its own C surface.
cat target/swift-bindings/pilotage_situation_ffiFFI.modulemap \
    target/swift-bindings/pilotage_instrument_apple_bridgeFFI.modulemap \
    > "$headers/module.modulemap"

xcodebuild -create-xcframework \
    -library target/aarch64-apple-ios/release/libpilotage_situation_ffi.a -headers "$headers" \
    -library target/aarch64-apple-ios-sim/release/libpilotage_situation_ffi.a -headers "$headers" \
    -library target/aarch64-apple-darwin/release/libpilotage_situation_ffi.a -headers "$headers" \
    -output "$artifact"

echo "built $artifact"
