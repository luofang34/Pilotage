#!/bin/sh
# Build review geometry from the committed Indicate instrument set.
set -eu
client_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
indicate_root=${INDICATE_ROOT:-"$client_root/../../../Indicate"}
revision=0e268cfc0826cc81ca9b4025299ff44fa178aa62
output_root="$client_root/.build/HwdReview"
source_root="$client_root/.build/hwd-review-source/$revision"
if [ ! -f "$source_root/.complete" ]; then
    archive=$(mktemp -d)
    trap 'rm -rf "$archive"' EXIT HUP INT TERM
    git -C "$indicate_root" archive "$revision" > "$archive/source.tar"
    mkdir -p "$source_root"
    tar -xf "$archive/source.tar" -C "$source_root"
    touch "$source_root/.complete"
fi
mkdir -p "$output_root"
CARGO_TARGET_DIR="$client_root/.build/hwd-review-target" cargo run --locked --release \
    --manifest-path "$source_root/Cargo.toml" -p hwd-bench -- "$output_root/hwd-gallery.html"
printf '%s\n' "$revision" > "$output_root/indicate-revision.txt"
