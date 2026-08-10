#!/bin/sh
# Build the pinned terrain renderer with the renderer's Apple package target.
set -eu

client_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
repository_root=$(CDPATH='' cd -- "$client_root/../.." && pwd)
source_root=${MAPLIBRE_TERRAIN_SOURCE:-"$(dirname -- "$repository_root")/maplibre-native"}
revision_file="$client_root/MAPLIBRE_TERRAIN_REVISION"
package_root="$client_root/Packages/PilotageMapLibreTerrain"
artifact_root="$package_root/Artifacts"
framework="$artifact_root/MapLibre.xcframework"
revision_marker="$artifact_root/REVISION"
build_target='//platform/ios:MapLibre.dynamic'
expected_revision=$(tr -d '[:space:]' < "$revision_file")

if [ -f "$framework/Info.plist" ] && [ -f "$revision_marker" ] && \
    [ "$(tr -d '[:space:]' < "$revision_marker")" = "$expected_revision" ]; then
    echo "MapLibre terrain artifact is ready at $expected_revision"
    exit 0
fi

if [ ! -e "$source_root/.git" ]; then
    echo "MAPLIBRE_TERRAIN_SOURCE must identify a MapLibre Native Git worktree" >&2
    exit 2
fi
if ! command -v bazel >/dev/null 2>&1; then
    echo "bazel is required to build the MapLibre terrain artifact" >&2
    exit 2
fi

actual_revision=$(git -C "$source_root" rev-parse HEAD)
if [ "$actual_revision" != "$expected_revision" ]; then
    echo "MapLibre Native must be at MAPLIBRE_TERRAIN_REVISION" >&2
    exit 2
fi
if [ -n "$(git -C "$source_root" status --porcelain --untracked-files=normal)" ]; then
    echo "MapLibre Native must have a clean worktree" >&2
    exit 2
fi
submodule_status=$(git -C "$source_root" submodule status --recursive)
if printf '%s\n' "$submodule_status" | grep -Eq '^[-+U]'; then
    echo "MapLibre Native submodules must match the pinned worktree" >&2
    exit 2
fi

(
    cd "$source_root"
    bazel build \
        --compilation_mode=opt \
        --features=dead_strip,thin_lto \
        --objc_enable_binary_stripping \
        --//:renderer=metal \
        "$build_target"
)

execution_root=$(cd "$source_root" && bazel info execution_root)
artifact_relative=$(
    cd "$source_root"
    bazel cquery \
        --output=files \
        --compilation_mode=opt \
        --//:renderer=metal \
        "$build_target"
)
artifact_archive="$execution_root/$artifact_relative"
if [ ! -f "$artifact_archive" ]; then
    echo "MapLibre Native did not produce an XCFramework archive" >&2
    exit 2
fi

temporary_root=$(mktemp -d "$package_root/Artifacts.stage.XXXXXX")
trap 'rm -rf "$temporary_root"' EXIT
unzip -q "$artifact_archive" -d "$temporary_root"
if [ ! -d "$temporary_root/MapLibre.xcframework" ]; then
    echo "the MapLibre archive has no MapLibre.xcframework" >&2
    exit 2
fi

rm -rf "$artifact_root"
mkdir -p "$artifact_root"
mv "$temporary_root/MapLibre.xcframework" "$framework"
cp "$source_root/LICENSE.md" "$artifact_root/LICENSE.md"
printf '%s\n' "$expected_revision" > "$revision_marker"
rm -rf "$temporary_root"
trap - EXIT
echo "built MapLibre terrain artifact at $expected_revision"
