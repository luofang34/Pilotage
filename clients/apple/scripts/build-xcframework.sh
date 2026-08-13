#!/bin/sh
# Build the Rust slices, Swift bindings, and XCFramework.
set -eu

client_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
ffi_root="$client_root/rust/pilotage-situation-ffi"
package_root="$client_root/Packages/PilotageCore"
cd "$ffi_root"

targets="aarch64-apple-ios aarch64-apple-ios-sim aarch64-apple-darwin"
for target in $targets; do
    if ! rustup target list --installed | grep -q "^$target$"; then
        rustup target add "$target"
    fi
    cargo build --release --target "$target"
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
cp target/swift-bindings/pilotage_situation_ffiFFI.h "$headers/"
cp target/swift-bindings/pilotage_situation_ffiFFI.modulemap "$headers/module.modulemap"

xcodebuild -create-xcframework \
    -library target/aarch64-apple-ios/release/libpilotage_situation_ffi.a -headers "$headers" \
    -library target/aarch64-apple-ios-sim/release/libpilotage_situation_ffi.a -headers "$headers" \
    -library target/aarch64-apple-darwin/release/libpilotage_situation_ffi.a -headers "$headers" \
    -output "$artifact"

echo "built $artifact"
