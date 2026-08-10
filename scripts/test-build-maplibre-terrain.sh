#!/usr/bin/env bash
# Prove the terrain build stages only the pinned MapLibre artifact.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT
client="$fixture/clients/apple-situation"
source_root="$fixture/maplibre-native"
mock_bin="$fixture/bin"
execution_root="$fixture/bazel-execution"
archive_source="$fixture/archive"
package_root="$client/Packages/PilotageMapLibreTerrain"

mkdir -p \
    "$client/scripts" \
    "$package_root" \
    "$source_root/.git" \
    "$mock_bin" \
    "$execution_root/out" \
    "$archive_source/MapLibre.xcframework"
cp "$root/clients/apple-situation/MAPLIBRE_TERRAIN_REVISION" "$client/"
cp "$root/clients/apple-situation/scripts/build-maplibre-terrain.sh" "$client/scripts/"
printf '%s\n' 'MapLibre test license' > "$source_root/LICENSE.md"
printf '%s\n' 'test framework' > "$archive_source/MapLibre.xcframework/Info.plist"
(
    cd "$archive_source"
    zip -qr "$execution_root/out/MapLibre.dynamic.xcframework.zip" MapLibre.xcframework
)

printf '%s\n' \
    '#!/bin/sh' \
    "case \"\$*\" in" \
    "    *\"rev-parse HEAD\"*) printf \"%s\\\\n\" \"\$MOCK_MAPLIBRE_REVISION\" ;;" \
    '    *"status --porcelain --untracked-files=normal"*) ;;' \
    '    *"submodule status --recursive"*) ;;' \
    "    *) echo \"unexpected git command: \$*\" >&2; exit 2 ;;" \
    'esac' \
    > "$mock_bin/git"
printf '%s\n' \
    '#!/bin/sh' \
    "case \"\$1\" in" \
    '    build) ;;' \
    "    info) printf \"%s\\\\n\" \"\$MOCK_BAZEL_EXECUTION_ROOT\" ;;" \
    '    cquery) printf "%s\\n" "out/MapLibre.dynamic.xcframework.zip" ;;' \
    "    *) echo \"unexpected bazel command: \$*\" >&2; exit 2 ;;" \
    'esac' \
    > "$mock_bin/bazel"
chmod +x "$mock_bin/git" "$mock_bin/bazel"

revision=$(tr -d '[:space:]' < "$client/MAPLIBRE_TERRAIN_REVISION")
PATH="$mock_bin:$PATH" \
MOCK_MAPLIBRE_REVISION="$revision" \
MOCK_BAZEL_EXECUTION_ROOT="$execution_root" \
MAPLIBRE_TERRAIN_SOURCE="$source_root" \
    sh "$client/scripts/build-maplibre-terrain.sh" >/dev/null

test -f "$package_root/Artifacts/MapLibre.xcframework/Info.plist"
test "$(tr -d '[:space:]' < "$package_root/Artifacts/REVISION")" = "$revision"
test -f "$package_root/Artifacts/LICENSE.md"

MAPLIBRE_TERRAIN_SOURCE="$fixture/missing" \
    sh "$client/scripts/build-maplibre-terrain.sh" >/dev/null

rm -rf "$package_root/Artifacts"
if PATH="$mock_bin:$PATH" \
    MOCK_MAPLIBRE_REVISION=0000000000000000000000000000000000000000 \
    MOCK_BAZEL_EXECUTION_ROOT="$execution_root" \
    MAPLIBRE_TERRAIN_SOURCE="$source_root" \
    sh "$client/scripts/build-maplibre-terrain.sh" >/dev/null 2>&1; then
    echo "the terrain build accepted a source at the wrong revision" >&2
    exit 1
fi

echo "MapLibre terrain build self-test: OK"
