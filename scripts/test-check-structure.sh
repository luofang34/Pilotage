#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
fixture="$(mktemp -d)"
trap 'rm -rf "$fixture"' EXIT

mkdir -p "$fixture/scripts" "$fixture/src" "$fixture/.build/vendor/src"
cp "$root_dir/scripts/check-structure.sh" "$fixture/scripts/"
git -C "$fixture" init -q

printf '%s\n' '.build/' > "$fixture/.gitignore"
printf '%s\n' '//! Test crate.' > "$fixture/src/lib.rs"
printf '%s\n' 'pub fn ignored_dependency() {}' \
    > "$fixture/.build/vendor/src/mod.rs"
git -C "$fixture" add .gitignore scripts/check-structure.sh src/lib.rs

printf '%s\n' 'pub fn deleted_module() {}' > "$fixture/src/mod.rs"
git -C "$fixture" add src/mod.rs
rm "$fixture/src/mod.rs"

bash "$fixture/scripts/check-structure.sh" --forbidden-filenames-only \
    >/dev/null

mkdir -p "$fixture/scratch"
printf '%s\n' 'pub fn first_party_module() {}' > "$fixture/scratch/mod.rs"
output="$fixture/failure.txt"
if bash "$fixture/scripts/check-structure.sh" --forbidden-filenames-only \
    >"$output" 2>&1; then
    echo "the structure guard accepted a nonignored mod.rs file" >&2
    exit 1
fi
if ! grep -qF './scratch/mod.rs' "$output"; then
    echo "the structure guard did not name the nonignored mod.rs file" >&2
    exit 1
fi

rm "$fixture/scratch/mod.rs"
cat > "$fixture/renderer.patch" <<'PATCH'
--- /dev/null
+++ b/maplibre/src/shading/mod.rs
@@ -0,0 +1 @@
+pub fn shade() {}
PATCH
if bash "$fixture/scripts/check-structure.sh" --forbidden-filenames-only \
    >"$output" 2>&1; then
    echo "the structure guard accepted mod.rs in a renderer patch" >&2
    exit 1
fi
if ! grep -qF 'maplibre/src/shading/mod.rs' "$output"; then
    echo "the structure guard did not name the patched mod.rs file" >&2
    exit 1
fi
cat > "$fixture/renderer.patch" <<'PATCH'
--- /dev/null
+++ b/maplibre/src/shading.rs
@@ -0,0 +1 @@
+pub fn shade() {}
PATCH
bash "$fixture/scripts/check-structure.sh" --forbidden-filenames-only >/dev/null

echo "check-structure self-test: OK"
