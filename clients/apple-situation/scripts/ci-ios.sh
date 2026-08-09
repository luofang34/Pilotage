#!/bin/sh
# Verify the Rust facade and the generated Swift package.
set -eu

client_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
manifest="$client_root/rust/pilotage-situation-ffi/Cargo.toml"
package="$client_root/Packages/PilotageSituationCore"
cache_root="$client_root/.build/cache"
mkdir -p "$cache_root/clang" "$cache_root/swiftpm"
export CLANG_MODULE_CACHE_PATH="$cache_root/clang"
export SWIFTPM_MODULECACHE_OVERRIDE="$cache_root/clang"
export XDG_CACHE_HOME="$cache_root/swiftpm"

cargo fmt --manifest-path "$manifest" --check
cargo clippy --manifest-path "$manifest" --locked --all-targets -- -D warnings
cargo test --manifest-path "$manifest" --locked
sh "$client_root/scripts/build-xcframework.sh"
swift build --disable-sandbox --package-path "$package"
swift test --disable-sandbox --package-path "$package"
