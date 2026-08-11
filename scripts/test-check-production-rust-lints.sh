#!/bin/sh
# Prove that the production Rust lint guard rejects a local lint exemption.
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
guard="$repo_root/scripts/check-production-rust-lints.sh"
fixture=$(mktemp -d)
trap 'rm -rf "$fixture"' EXIT

mkdir -p "$fixture/src"

cat > "$fixture/Cargo.toml" <<'EOF'
[package]
name = "production-lint-fixture"
version = "0.1.0"
edition = "2024"

[workspace]
EOF

cat > "$fixture/src/lib.rs" <<'EOF'
pub fn first(values: &[u8]) -> Option<u8> {
    values.first().copied()
}
EOF

if ! sh "$guard" "$fixture" >/dev/null 2>&1; then
    echo "the production Rust lint guard rejected valid code" >&2
    exit 1
fi

cat > "$fixture/src/lib.rs" <<'EOF'
#[allow(clippy::expect_used)]
pub fn first(values: &[u8]) -> u8 {
    *values.first().expect("a value")
}
EOF

if output=$(sh "$guard" "$fixture" 2>&1); then
    echo "the production Rust lint guard accepted an expect exemption" >&2
    exit 1
fi
case "$output" in
    *expect_used*) ;;
    *)
        echo "the production Rust lint guard failed for an unrelated reason" >&2
        echo "$output" >&2
        exit 1
        ;;
esac

echo "production Rust lint guard self-test: OK"
