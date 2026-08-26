#!/usr/bin/env bash
# Vendor the MapLibre GL JS renderer into the web client.
#
# The web client loads no external resource at run time. The renderer must be
# a local file, so this script downloads one pinned release archive, verifies
# its digest, and copies the four runtime files into clients/web/vendor/.
# The vendor directory is a build artifact and is not committed.
set -euo pipefail

MAPLIBRE_VERSION="6.6.0"
# Digest of the pinned npm release archive.
MAPLIBRE_TARBALL_SHA256="d329c597381ab52589260d89914c80fb53ef32ba07647e9f2c71f58fdf7b606e"
MAPLIBRE_TARBALL_URL="https://registry.npmjs.org/maplibre-gl/-/maplibre-gl-${MAPLIBRE_VERSION}.tgz"

# The ESM bundle resolves its two sibling chunks against import.meta.url, so
# all four files must sit in one directory.
RUNTIME_FILES=(
    "maplibre-gl.mjs"
    "maplibre-gl-shared.mjs"
    "maplibre-gl-worker.mjs"
    "maplibre-gl.css"
)

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dest="$root/clients/web/vendor/maplibre-gl"
stamp="$dest/VENDOR.json"

# The fast path accepts an existing vendor directory only when every
# runtime file still matches the digest the stamp recorded at install.
if [ -f "$stamp" ] && python3 - "$stamp" "$dest" "$MAPLIBRE_VERSION" <<'EOF'
import hashlib
import json
import sys
from pathlib import Path

stamp_path, dest, version = sys.argv[1:4]
try:
    with open(stamp_path, encoding="utf-8") as handle:
        record = json.load(handle)
except (OSError, ValueError):
    sys.exit(1)
files = record.get("files")
if record.get("version") != version or not isinstance(files, dict) or not files:
    sys.exit(1)
for name, digest in files.items():
    try:
        actual = hashlib.sha256((Path(dest) / name).read_bytes()).hexdigest()
    except OSError:
        sys.exit(1)
    if actual != digest:
        sys.exit(1)
EOF
then
    echo "maplibre-gl ${MAPLIBRE_VERSION} already vendored at $dest"
    exit 0
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

echo "Downloading maplibre-gl ${MAPLIBRE_VERSION} ..."
curl -fsSL -o "$work/maplibre-gl.tgz" "$MAPLIBRE_TARBALL_URL"

actual_sha256="$(shasum -a 256 "$work/maplibre-gl.tgz" | awk '{print $1}')"
if [ "$actual_sha256" != "$MAPLIBRE_TARBALL_SHA256" ]; then
    echo "FORBIDDEN: maplibre-gl archive digest mismatch" >&2
    echo "  expected: $MAPLIBRE_TARBALL_SHA256" >&2
    echo "  actual:   $actual_sha256" >&2
    exit 1
fi

tar -xzf "$work/maplibre-gl.tgz" -C "$work"
mkdir -p "$dest"
for file in "${RUNTIME_FILES[@]}"; do
    if [ ! -f "$work/package/dist/$file" ]; then
        echo "FORBIDDEN: release archive lacks dist/$file" >&2
        exit 1
    fi
    cp "$work/package/dist/$file" "$dest/$file"
done
cp "$work/package/LICENSE.txt" "$dest/LICENSE.txt"

python3 - "$stamp" "$dest" "$MAPLIBRE_VERSION" "$MAPLIBRE_TARBALL_SHA256" \
    "${RUNTIME_FILES[@]}" <<'EOF'
import hashlib
import json
import sys
from pathlib import Path

path, dest, version, sha256 = sys.argv[1:5]
files = {
    name: hashlib.sha256((Path(dest) / name).read_bytes()).hexdigest()
    for name in sys.argv[5:]
}
with open(path, "w", encoding="utf-8") as handle:
    json.dump(
        {
            "name": "maplibre-gl",
            "version": version,
            "tarball_sha256": sha256,
            "files": files,
        },
        handle,
        indent=2,
        sort_keys=True,
    )
    handle.write("\n")
EOF

echo "Vendored maplibre-gl ${MAPLIBRE_VERSION} into $dest"
