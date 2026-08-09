#!/usr/bin/env bash
# Verifies the Pilotage pin against the release manifest at the pinned revision.
set -euo pipefail
cd "$(dirname "$0")/.."

pin="clients/instrument-compatibility.json"
revision="$(jq -r '.indicateRevision' "$pin")"
manifest="$(mktemp "${TMPDIR:-/tmp}/indicate-release-manifest.XXXXXX")"
trap 'rm -f "$manifest"' EXIT

checkout="${INDICATE_CHECKOUT:-}"
if [ -n "$checkout" ] && [ -f "$checkout/release-manifest.json" ] && \
   [ "$(git -C "$checkout" rev-parse HEAD)" = "$revision" ]; then
  cp "$checkout/release-manifest.json" "$manifest"
else
  curl --fail --location --retry 3 --silent --show-error \
    "https://raw.githubusercontent.com/luofang34/Indicate/$revision/release-manifest.json" \
    --output "$manifest"
fi

check_equal() {
  local label="$1"
  local expected="$2"
  local actual="$3"
  if [ "$expected" != "$actual" ]; then
    echo "$label mismatch: pin=$expected manifest=$actual" >&2
    return 1
  fi
}

check_equal "state ABI" \
  "$(jq -r '.stateAbiVersion' "$pin")" \
  "$(jq -r '.stateAbi.version' "$manifest")"
check_equal "scene format" \
  "$(jq -r '.sceneFormatVersion' "$pin")" \
  "$(jq -r '.sceneFormatVersion' "$manifest")"
check_equal "corpus version" \
  "$(jq -r '.corpusVersion' "$pin")" \
  "$(jq -r '.corpus.version' "$manifest")"
check_equal "corpus digest" \
  "$(jq -r '.corpusDigest' "$pin")" \
  "$(jq -r '.corpus.sha256' "$manifest")"
check_equal "registry scene digest" \
  "$(jq -r '.registrySceneDigest' "$pin")" \
  "$(jq -r '.compositionDigest' "$manifest")"
check_equal "glyph pack hash" \
  "$(jq -r '.glyphRecordedHash' "$pin")" \
  "$(jq -r '.glyphPackHash' "$manifest")"

cargo_revisions="$(
  sed -nE 's/.*Indicate\.git", rev = "([0-9a-f]+)".*/\1/p' Cargo.toml |
    sort -u
)"
if [ "$cargo_revisions" != "$revision" ]; then
  echo "Indicate Cargo revisions do not equal the compatibility pin" >&2
  exit 1
fi

cargo test --locked -p pilotage-instrument-runtime compatibility_pin_matches_linked_runtime
echo "Indicate compatibility pin matches revision $revision"
