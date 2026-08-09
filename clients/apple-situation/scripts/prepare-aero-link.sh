#!/bin/sh
# Stage the pinned AeroLink source for this application.
set -eu

client_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
repository_root=$(CDPATH='' cd -- "$client_root/../.." && pwd)
source_root=${AERO_LINK_SOURCE:-"$(dirname -- "$repository_root")/aero-link"}
revision_file="$client_root/AERO_LINK_REVISION"
stage_root="$client_root/.build/aero-link"
temporary_stage="$client_root/.build/aero-link-stage"

if [ ! -d "$source_root/.git" ]; then
    echo "AERO_LINK_SOURCE must identify an AeroLink Git worktree" >&2
    exit 2
fi

expected_revision=$(tr -d '[:space:]' < "$revision_file")
actual_revision=$(git -C "$source_root" rev-parse HEAD)
if [ "$actual_revision" != "$expected_revision" ]; then
    echo "AeroLink HEAD does not match AERO_LINK_REVISION" >&2
    exit 2
fi

rm -rf "$stage_root" "$temporary_stage"
mkdir -p "$temporary_stage"
git -C "$source_root" archive "$expected_revision" | tar -x -C "$temporary_stage"
mv "$temporary_stage" "$stage_root"
echo "staged AeroLink $expected_revision"
