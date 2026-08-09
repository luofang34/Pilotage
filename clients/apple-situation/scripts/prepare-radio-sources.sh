#!/bin/sh
# Stage the exact radio-domain sources for the standalone Rust facade.
set -eu

client_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
repository_root=$(CDPATH='' cd -- "$client_root/../.." && pwd)
source_parent=$(dirname -- "$repository_root")
airmass_source=${AIRMASS_SOURCE:-"$source_parent/Airmass"}
surveillance_source=${SURVEILLANCE_SOURCE:-"$source_parent/Surveillance"}

stage_crates() {
    source_root=$1
    revision_file=$2
    stage_name=$3
    product_name=$4
    shift 4
    facade_root="$client_root/rust/pilotage-situation-ffi"
    stage_root="$facade_root/.build/$stage_name"
    revision=$(tr -d '[:space:]' < "$revision_file")

    if [ ! -d "$source_root/.git" ]; then
        echo "$product_name source must identify a Git worktree" >&2
        exit 2
    fi
    if ! git -C "$source_root" cat-file -e "$revision^{commit}"; then
        echo "$product_name source does not contain the required revision" >&2
        exit 2
    fi

    mkdir -p "$facade_root/.build"
    temporary_stage=$(mktemp -d "$facade_root/.build/$stage_name-stage.XXXXXX")
    git -C "$source_root" archive "$revision" "$@" | tar -x -C "$temporary_stage"
    rm -rf "$stage_root"
    mv "$temporary_stage" "$stage_root"
    echo "staged $product_name $revision"
}

mkdir -p "$client_root/.build"
sh "$client_root/scripts/prepare-aero-link.sh"
stage_crates \
    "$airmass_source" \
    "$client_root/AIRMASS_REVISION" \
    "airmass" \
    "Airmass" \
    crates/airmass-aero-link \
    crates/airmass-core \
    crates/airmass-geojson
stage_crates \
    "$surveillance_source" \
    "$client_root/SURVEILLANCE_REVISION" \
    "surveillance" \
    "Surveillance" \
    crates/surveillance-aero-link \
    crates/surveillance-core \
    crates/surveillance-geojson
