#!/usr/bin/env bash
# Prove that the tuning boundary guard rejects simulator-specific coupling.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
guard="$repo_root/scripts/check-flight-tune-boundaries.sh"
fixture="$(mktemp -d "${TMPDIR:-/tmp}/pilotage-flight-tune-boundary-test.XXXXXX")"
fixture_allowlist="$fixture/import-allowlist.tsv"
trap 'rm -rf "$fixture"' EXIT

mkdir -p \
    "$fixture/adapters/flight-tune-xplane/src" \
    "$fixture/adapters/pilotage-xplane-trial/src" \
    "$fixture/crates/pilotage-trial/src" \
    "$fixture/injected/src" \
    "$fixture/scripts" \
    "$fixture/tools/flight-tune/src" \
    "$fixture/tools/flight-tune-aviate/src/bin" \
    "$fixture/tools/flight-tune-aviate/src/support" \
    "$fixture/tools/flight-tune-aviate/src/tests" \
    "$fixture/tools/flight-tune-campaign/src/config" \
    "$fixture/vendor/errno/src" \
    "$fixture/vendor/libproc/src" \
    "$fixture/vendor/sysctl/src"

write_workspace() {
    printf '%s\n' \
        '[workspace]' \
        'members = [' \
        '    "adapters/flight-tune-xplane",' \
        '    "adapters/pilotage-xplane-trial",' \
        '    "crates/pilotage-trial",' \
        '    "injected",' \
        '    "tools/flight-tune",' \
        '    "tools/flight-tune-aviate",' \
        '    "tools/flight-tune-campaign",' \
        '    "vendor/errno",' \
        '    "vendor/libproc",' \
        '    "vendor/sysctl",' \
        ']' \
        'resolver = "2"' \
        > "$fixture/Cargo.toml"
}

write_adapter_packages() {
    printf '%s\n' \
        '[package]' \
        'name = "flight-tune-xplane"' \
        'version = "0.0.0"' \
        'edition = "2021"' \
        > "$fixture/adapters/flight-tune-xplane/Cargo.toml"
    printf '%s\n' 'pub fn adapter() {}' \
        > "$fixture/adapters/flight-tune-xplane/src/lib.rs"
    printf '%s\n' \
        '[package]' \
        'name = "pilotage-xplane-trial"' \
        'version = "0.0.0"' \
        'edition = "2021"' \
        > "$fixture/adapters/pilotage-xplane-trial/Cargo.toml"
    printf '%s\n' 'pub fn adapter() {}' \
        > "$fixture/adapters/pilotage-xplane-trial/src/lib.rs"
    printf '%s\n' \
        '[package]' \
        'name = "injected"' \
        'version = "0.0.0"' \
        'edition = "2021"' \
        '[lib]' \
        'proc-macro = true' \
        > "$fixture/injected/Cargo.toml"
    printf '%s\n' 'extern crate proc_macro;' \
        > "$fixture/injected/src/lib.rs"
    printf '%s\n' \
        '[package]' \
        'name = "errno"' \
        'version = "0.3.14"' \
        'edition = "2021"' \
        > "$fixture/vendor/errno/Cargo.toml"
    printf '%s\n' 'pub fn fixture() {}' \
        > "$fixture/vendor/errno/src/lib.rs"
    printf '%s\n' \
        '[package]' \
        'name = "libproc"' \
        'version = "0.14.11"' \
        'edition = "2021"' \
        > "$fixture/vendor/libproc/Cargo.toml"
    printf '%s\n' 'pub fn fixture() {}' \
        > "$fixture/vendor/libproc/src/lib.rs"
    printf '%s\n' \
        '[package]' \
        'name = "sysctl"' \
        'version = "0.7.1"' \
        'edition = "2021"' \
        > "$fixture/vendor/sysctl/Cargo.toml"
    printf '%s\n' 'pub fn fixture() {}' \
        > "$fixture/vendor/sysctl/src/lib.rs"
}

write_clean_cargo_config() {
    mkdir -p "$fixture/.cargo"
    printf '%s\n' \
        '[env]' \
        'BINDGEN_EXTRA_CLANG_ARGS_aarch64_apple_darwin = "--target=aarch64-apple-darwin"' \
        > "$fixture/.cargo/config.toml"
}

write_aviate_manifest() {
    local errno_version="$1" libproc_version="$2" sysctl_version="$3"
    local normal_dependency="${4:-}"
    {
        printf '%s\n' \
            '[package]' \
            'name = "flight-tune-aviate"' \
            'version = "0.0.0"' \
            'edition = "2021"' \
            '[dependencies]'
        if [ -n "$normal_dependency" ]; then
            printf '%s\n' "$normal_dependency"
        fi
        printf '%s\n' \
            '[target.'\''cfg(target_os = "macos")'\''.dependencies]' \
            "errno = { version = \"$errno_version\", path = \"../../vendor/errno\" }" \
            "libproc = { version = \"$libproc_version\", path = \"../../vendor/libproc\" }" \
            "sysctl = { version = \"$sysctl_version\", path = \"../../vendor/sysctl\" }"
    } > "$fixture/tools/flight-tune-aviate/Cargo.toml"
}

write_clean_manifests() {
    printf '%s\n' \
        '[package]' \
        'name = "pilotage-trial"' \
        'version = "0.0.0"' \
        'edition = "2021"' \
        '[dependencies]' \
        > "$fixture/crates/pilotage-trial/Cargo.toml"
    printf '%s\n' \
        '[package]' \
        'name = "flight-tune"' \
        'version = "0.0.0"' \
        'edition = "2021"' \
        '[dependencies]' \
        > "$fixture/tools/flight-tune/Cargo.toml"
    write_aviate_manifest '=0.3.14' '=0.14.11' '=0.7.1'
    printf '%s\n' 'pub fn host() {}' \
        > "$fixture/tools/flight-tune-aviate/src/lib.rs"
    printf '%s\n' \
        '[package]' \
        'name = "flight-tune-campaign"' \
        'version = "0.0.0"' \
        'edition = "2021"' \
        '[dependencies]' \
        > "$fixture/tools/flight-tune-campaign/Cargo.toml"
    printf '%s\n' 'pub mod config;' \
        > "$fixture/tools/flight-tune-campaign/src/lib.rs"
}

write_dependency_allowlist() {
    local include_xplane="${1:-false}"
    {
        printf '%s\n' \
            $'manifest\tdependency_key\tactual_package\tkind\ttarget\toptional\tsource_kind\tsource_ref\tdefault_features\tfeatures\tversion_req' \
            $'tools/flight-tune-aviate/Cargo.toml\terrno\terrno\tnormal\tcfg(target_os = "macos")\tfalse\tpath\tvendor/errno\ttrue\t\t=0.3.14'
        if [ "$include_xplane" = true ]; then
            printf '%s\n' \
                $'tools/flight-tune-aviate/Cargo.toml\tflight-tune-xplane\tflight-tune-xplane\tnormal\t\tfalse\tpath\tadapters/flight-tune-xplane\ttrue\t\t*'
        fi
        printf '%s\n' \
            $'tools/flight-tune-aviate/Cargo.toml\tlibproc\tlibproc\tnormal\tcfg(target_os = "macos")\tfalse\tpath\tvendor/libproc\ttrue\t\t=0.14.11' \
            $'tools/flight-tune-aviate/Cargo.toml\tsysctl\tsysctl\tnormal\tcfg(target_os = "macos")\tfalse\tpath\tvendor/sysctl\ttrue\t\t=0.7.1'
    } > "$fixture/scripts/flight-tune-direct-dependency-allowlist.tsv"
}

write_allowlisted_import() {
    printf '%s\n' \
        'use flight_tune_xplane::CausalJoinConfig;' \
        'pub fn marker() {}' \
        > "$fixture/tools/flight-tune-aviate/src/driver.rs"
}

write_shared_contract() {
    printf '%s\n' \
        'pub struct SharedContract {' \
        '    pub id: String,' \
        '}' \
        > "$fixture/crates/pilotage-trial/src/lib.rs"
    printf '%s\n' \
        'pub struct RuntimeContract {' \
        '    pub id: String,' \
        '}' \
        > "$fixture/tools/flight-tune/src/lib.rs"
}

write_campaign_contract() {
    printf '%s\n' \
        'pub enum ExecutionTarget { Simulator, Hardware }' \
        'pub enum CampaignPurpose { Qualification, Canary }' \
        'pub struct PinnedFile { pub path: String }' \
        'pub struct SearchGroupConfig { pub id: String }' \
        'pub enum SearchGroupKind { Controller, Feel }' \
        'pub struct CampaignConfig {' \
        '    pub id: String,' \
        '}' \
        'pub struct XPlaneCampaignConfig {' \
        '    pub weather_plugin_digest: String,' \
        '    pub sdk_version: String,' \
        '}' \
        > "$fixture/tools/flight-tune-campaign/src/config.rs"
    printf '%s\n' \
        'pub struct CampaignBudgetLimit { pub attempts: u64 }' \
        'pub struct TrainingGuardScenarioConfig { pub id: String }' \
        'pub struct TrainingSuiteConfig { pub id: String }' \
        > "$fixture/tools/flight-tune-campaign/src/config/training_suite.rs"
}

run_guard() {
    bash "$guard" "$fixture" "$fixture_allowlist"
}

expect_failure() {
    local name="$1" expected="$2" output
    if output="$(run_guard 2>&1)"; then
        echo "the tuning boundary guard accepted $name" >&2
        exit 1
    fi
    if ! grep -Fq "$expected" <<<"$output"; then
        echo "the tuning boundary guard gave the wrong result for $name" >&2
        echo "$output" >&2
        exit 1
    fi
}

expect_failure_with_all() {
    local name="$1" output expected
    shift
    if output="$(run_guard 2>&1)"; then
        echo "the tuning boundary guard accepted $name" >&2
        exit 1
    fi
    for expected in "$@"; do
        if ! grep -Fq "$expected" <<<"$output"; then
            echo "the tuning boundary guard missed $expected for $name" >&2
            echo "$output" >&2
            exit 1
        fi
    done
}

write_workspace
write_adapter_packages
write_clean_cargo_config
write_clean_manifests
write_dependency_allowlist
write_allowlisted_import
write_shared_contract
write_campaign_contract
printf '%s\t%s\n' \
    'tools/flight-tune-aviate/src/driver.rs' \
    'useflight_tune_xplane::CausalJoinConfig;' \
    > "$fixture_allowlist"
run_guard >/dev/null

printf '%s\n' '' '[patch.crates-io]' \
    'serde = { path = "crates/pilotage-trial" }' \
    >> "$fixture/Cargo.toml"
expect_failure \
    'a workspace Cargo patch source override' \
    'Cargo.toml has an unreviewed dependency source override'
write_workspace

mkdir -p "$fixture/.cargo"
printf '%s\n' 'paths = ["../injected"]' \
    > "$fixture/.cargo/config.toml"
expect_failure \
    'a Cargo configuration path source override' \
    '.cargo/config.toml has an unreviewed dependency source override'
write_clean_cargo_config

printf '%s\n' 'include = ["source-override.toml"]' \
    > "$fixture/.cargo/config.toml"
printf '%s\n' \
    '[source.crates-io]' \
    'replace-with = "local-registry"' \
    > "$fixture/.cargo/source-override.toml"
expect_failure \
    'an included Cargo configuration source override' \
    '.cargo/config.toml has an unreviewed dependency source override'
rm "$fixture/.cargo/source-override.toml"
write_clean_cargo_config

printf '%s\n' \
    '[env]' \
    'BINDGEN_EXTRA_CLANG_ARGS_AARCH64_APPLE_DARWIN = "--target=aarch64-apple-darwin"' \
    > "$fixture/.cargo/config.toml"
expect_failure \
    'a changed macOS bindgen environment key' \
    '.cargo/config.toml must set BINDGEN_EXTRA_CLANG_ARGS_aarch64_apple_darwin'
write_clean_cargo_config

printf '%s\n' \
    '[env]' \
    'BINDGEN_EXTRA_CLANG_ARGS_aarch64_apple_darwin = "--target=x86_64-apple-darwin"' \
    > "$fixture/.cargo/config.toml"
expect_failure \
    'a changed macOS bindgen environment value' \
    '.cargo/config.toml must set BINDGEN_EXTRA_CLANG_ARGS_aarch64_apple_darwin'
write_clean_cargo_config

printf '%s\n' \
    'version = 4' \
    '' \
    '[[package]]' \
    'name = "serde"' \
    'version = "999.0.0"' \
    'source = "registry+https://github.com/rust-lang/crates.io-index"' \
    'checksum = "0000000000000000000000000000000000000000000000000000000000000000"' \
    > "$fixture/Cargo.lock"
expect_failure \
    'an unreviewed Cargo lockfile registry identity' \
    'Cargo.lock has an unreviewed registry identity for serde'
rm "$fixture/Cargo.lock"

printf '%s\n' \
    $'manifest\tdependency_key\tactual_package\tkind\ttarget\toptional\tsource_kind\tsource_ref\tdefault_features\tfeatures\tversion_req' \
    $'tools/flight-tune-aviate/Cargo.toml\tsysctl\tsysctl\tnormal\tcfg(target_os = "macos")\tfalse\tpath\tvendor/sysctl\ttrue\t\t=0.7.1' \
    $'tools/flight-tune-aviate/Cargo.toml\terrno\terrno\tnormal\tcfg(target_os = "macos")\tfalse\tpath\tvendor/errno\ttrue\t\t=0.3.14' \
    $'tools/flight-tune-aviate/Cargo.toml\tlibproc\tlibproc\tnormal\tcfg(target_os = "macos")\tfalse\tpath\tvendor/libproc\ttrue\t\t=0.14.11' \
    > "$fixture/scripts/flight-tune-direct-dependency-allowlist.tsv"
expect_failure \
    'an unsorted direct dependency allowlist' \
    'direct dependency allowlist is not sorted or has a duplicate'
write_dependency_allowlist

printf '%s\n' \
    $'tools/flight-tune-aviate/Cargo.toml\terrno\terrno\tnormal\tcfg(target_os = "macos")\tfalse\tpath\tvendor/errno\ttrue\t\t=0.3.14' \
    >> "$fixture/scripts/flight-tune-direct-dependency-allowlist.tsv"
expect_failure \
    'a duplicate direct dependency allowlist record' \
    'direct dependency allowlist is not sorted or has a duplicate'
write_dependency_allowlist

write_aviate_manifest '^0.3.14' '=0.14.11' '=0.7.1'
expect_failure \
    'a non-exact macOS errno pin' \
    'has unreviewed direct production dependency errno'
write_aviate_manifest '=0.3.14' '^0.14.11' '=0.7.1'
expect_failure \
    'a non-exact macOS libproc pin' \
    'has unreviewed direct production dependency libproc'
write_aviate_manifest '=0.3.14' '=0.14.11' '^0.7.1'
expect_failure \
    'a non-exact macOS sysctl pin' \
    'has unreviewed direct production dependency sysctl'
write_aviate_manifest '=0.3.14' '=0.14.11' '=0.7.1'

write_aviate_manifest \
    '=0.3.14' \
    '=0.14.11' \
    '=0.7.1' \
    'flight-tune-xplane = { path = "../../adapters/flight-tune-xplane" }'
write_dependency_allowlist true
expect_failure \
    'a reviewed Aviate flight-tune-xplane runtime dependency' \
    'noncanonical runtime dependency flight-tune-xplane'
write_clean_manifests
write_dependency_allowlist

printf '%s\n' 'pub fn marker() {}' \
    > "$fixture/tools/flight-tune-aviate/src/driver.rs"
run_guard >/dev/null
write_allowlisted_import

printf '%s\n' 'use flight_tune_xplane::ScenarioRuntime;' \
    > "$fixture/tools/flight-tune-aviate/src/driver.rs"
expect_failure \
    'a changed allowlisted import' \
    'new flight_tune_xplane import:'
write_allowlisted_import

printf '%s\n' 'use flight_tune_xplane::NewRuntimeType;' \
    > "$fixture/tools/flight-tune-aviate/src/new_runtime.rs"
expect_failure \
    'a new Aviate X-Plane import' \
    'new flight_tune_xplane import:'
rm "$fixture/tools/flight-tune-aviate/src/new_runtime.rs"

printf '%s\n' 'use flight_tune_xplane::BinRuntimeType;' \
    > "$fixture/tools/flight-tune-aviate/src/bin/runtime.rs"
expect_failure \
    'an import in a production bin module' \
    'new flight_tune_xplane import:'
rm "$fixture/tools/flight-tune-aviate/src/bin/runtime.rs"

printf '%s\n' 'pub use flight_tune_xplane::SupportRuntimeType;' \
    > "$fixture/tools/flight-tune-aviate/src/support/runtime.rs"
expect_failure \
    'an import in a production support module' \
    'new flight_tune_xplane import:'
rm "$fixture/tools/flight-tune-aviate/src/support/runtime.rs"

printf '%s\n' 'use flight_tune_xplane::TestOnlyType;' \
    > "$fixture/tools/flight-tune-aviate/src/tests/runtime.rs"
printf '%s\n' 'use flight_tune_xplane::TestSupportType;' \
    > "$fixture/tools/flight-tune-aviate/src/test_support.rs"
run_guard >/dev/null

printf '%s\n' \
    '#[cfg(test)]' \
    '#[path = "tests/runtime.rs"]' \
    'mod runtime_fixture;' \
    > "$fixture/tools/flight-tune-aviate/src/lib.rs"
run_guard >/dev/null

printf '%s\n' \
    '#[path = "tests/runtime.rs"]' \
    'mod runtime_fixture;' \
    > "$fixture/tools/flight-tune-aviate/src/lib.rs"
expect_failure \
    'an unguarded test-path module' \
    'path attribute imports test-only source without cfg(test)'

printf '%s\n' 'mod test_support;' \
    > "$fixture/tools/flight-tune-aviate/src/lib.rs"
expect_failure \
    'an unguarded test-support module' \
    'module test_support is not restricted to cfg(test)'
write_clean_manifests

printf '%s\n' \
    '[package]' \
    'name = "flight-tune-aviate"' \
    'version = "0.0.0"' \
    'edition = "2021"' \
    '[lib]' \
    'path = "src/tests/runtime.rs"' \
    '[dependencies]' \
    'flight-tune-xplane = { path = "../../adapters/flight-tune-xplane" }' \
    'pilotage-xplane-trial = { path = "../../adapters/pilotage-xplane-trial" }' \
    > "$fixture/tools/flight-tune-aviate/Cargo.toml"
expect_failure \
    'a production Cargo target in a test-only path' \
    'has a production target outside its scanned source root'
write_clean_manifests

printf '%s\n' \
    '[package]' \
    'name = "flight-tune-aviate"' \
    'version = "0.0.0"' \
    'edition = "2021"' \
    '[lib]' \
    'path = "src/tests/runtime.rs"' \
    'crate-type = ["cdylib"]' \
    '[dependencies]' \
    'flight-tune-xplane = { path = "../../adapters/flight-tune-xplane" }' \
    'pilotage-xplane-trial = { path = "../../adapters/pilotage-xplane-trial" }' \
    > "$fixture/tools/flight-tune-aviate/Cargo.toml"
expect_failure \
    'a production cdylib target in a test-only path' \
    'has a production target outside its scanned source root'
write_clean_manifests

printf '%s\n' 'pub fn runtime() { flight_tune_xplane::run(); }' \
    > "$fixture/tools/flight-tune-aviate/src/runtime.rs"
expect_failure \
    'a path reference outside a reviewed import' \
    'uses flight_tune_xplane outside a reviewed import'
rm "$fixture/tools/flight-tune-aviate/src/runtime.rs"

printf '%s\n' 'use flight_tune_xplane::SymlinkRuntime;' \
    > "$fixture/symlink-runtime.rs"
ln -s "$fixture/symlink-runtime.rs" \
    "$fixture/tools/flight-tune-aviate/src/symlink_runtime.rs"
expect_failure \
    'a production Aviate source symlink' \
    'production Rust source path tools/flight-tune-aviate/src/symlink_runtime.rs is a symlink'
rm "$fixture/tools/flight-tune-aviate/src/symlink_runtime.rs"

printf '%s\n' \
    '[package]' \
    'name = "flight-tune-aviate"' \
    'version = "0.0.0"' \
    'edition = "2021"' \
    '[dependencies]' \
    'xplane-adapter = { package = "flight-tune-xplane", path = "../../adapters/flight-tune-xplane" }' \
    'pilotage-xplane-trial = { path = "../../adapters/pilotage-xplane-trial" }' \
    > "$fixture/tools/flight-tune-aviate/Cargo.toml"
expect_failure \
    'a renamed Aviate X-Plane dependency' \
    'noncanonical runtime dependency flight-tune-xplane'
write_clean_manifests

printf '%s\n' \
    '[package]' \
    'name = "flight-tune-aviate"' \
    'version = "0.0.0"' \
    'edition = "2021"' \
    '[dependencies]' \
    'flight-tune-xplane = { path = "../../adapters/flight-tune-xplane", optional = true }' \
    > "$fixture/tools/flight-tune-aviate/Cargo.toml"
expect_failure \
    'an optional Aviate X-Plane dependency' \
    'noncanonical runtime dependency flight-tune-xplane'
write_clean_manifests

printf '%s\n' \
    '[package]' \
    'name = "flight-tune-aviate"' \
    'version = "0.0.0"' \
    'edition = "2021"' \
    '[build-dependencies]' \
    'flight-tune-xplane = { path = "../../adapters/flight-tune-xplane" }' \
    > "$fixture/tools/flight-tune-aviate/Cargo.toml"
expect_failure \
    'an Aviate X-Plane build dependency' \
    'noncanonical runtime dependency flight-tune-xplane'
write_clean_manifests

printf '%s\n' \
    '[package]' \
    'name = "flight-tune-aviate"' \
    'version = "0.0.0"' \
    'edition = "2021"' \
    '[target."cfg(unix)".dependencies]' \
    'flight-tune-xplane = { path = "../../adapters/flight-tune-xplane" }' \
    > "$fixture/tools/flight-tune-aviate/Cargo.toml"
expect_failure \
    'a target-specific Aviate X-Plane dependency' \
    'noncanonical runtime dependency flight-tune-xplane'
write_clean_manifests

cp "$fixture_allowlist" \
    "$fixture/scripts/flight-tune-xplane-import-allowlist.tsv"
bash "$guard" "$fixture" >/dev/null

mkdir -p "$fixture/failing-bin"
printf '%s\n' '#!/usr/bin/env bash' 'exit 9' > "$fixture/failing-bin/find"
chmod +x "$fixture/failing-bin/find"
if output="$(PATH="$fixture/failing-bin:$PATH" run_guard 2>&1)"; then
    echo 'the tuning boundary guard ignored a source-list failure' >&2
    exit 1
fi
if ! grep -Fq 'cannot list Rust source files below' <<<"$output"; then
    echo 'the tuning boundary guard did not report a source-list failure' >&2
    exit 1
fi
rm "$fixture/failing-bin/find"
printf '%s\n' '#!/usr/bin/env bash' 'exit 9' > "$fixture/failing-bin/grep"
chmod +x "$fixture/failing-bin/grep"
if output="$(PATH="$fixture/failing-bin:$PATH" run_guard 2>&1)"; then
    echo 'the tuning boundary guard ignored an import-scan failure' >&2
    exit 1
fi
if ! grep -Fq 'cannot be scanned for flight_tune_xplane references' \
    <<<"$output"; then
    echo 'the tuning boundary guard did not report an import-scan failure' >&2
    exit 1
fi
rm "$fixture/failing-bin/grep"
printf '%s\n' '#!/usr/bin/env bash' 'exit 9' > "$fixture/failing-bin/comm"
chmod +x "$fixture/failing-bin/comm"
if output="$(PATH="$fixture/failing-bin:$PATH" run_guard 2>&1)"; then
    echo 'the tuning boundary guard ignored an import comparison failure' >&2
    exit 1
fi
if ! grep -Fq 'cannot compare flight_tune_xplane imports with the allowlist' \
    <<<"$output"; then
    echo 'the tuning boundary guard did not report an import comparison failure' >&2
    exit 1
fi
rm "$fixture/failing-bin/comm"

printf '%s\n' \
    '[package]' \
    'name = "flight-tune"' \
    'version = "0.0.0"' \
    'edition = "2021"' \
    '[dependencies]' \
    'flight-tune-xplane = { path = "../../adapters/flight-tune-xplane" }' \
    > "$fixture/tools/flight-tune/Cargo.toml"
expect_failure \
    'a direct flight-tune-xplane runtime dependency' \
    'runtime dependency flight-tune-xplane'
write_clean_manifests

printf '%s\n' \
    '[package]' \
    'name = "flight-tune"' \
    'version = "0.0.0"' \
    'edition = "2021"' \
    '[dependencies]' \
    'adapter = { package = "flight-tune-xplane", path = "../../adapters/flight-tune-xplane" }' \
    > "$fixture/tools/flight-tune/Cargo.toml"
expect_failure \
    'an aliased flight-tune-xplane runtime dependency' \
    'runtime dependency flight-tune-xplane'
write_clean_manifests

printf '%s\n' \
    '[package]' \
    'name = "flight-tune"' \
    'version = "0.0.0"' \
    'edition = "2021"' \
    '[dependencies]' \
    'pilotage-xplane-trial = { path = "../../adapters/pilotage-xplane-trial" }' \
    > "$fixture/tools/flight-tune/Cargo.toml"
expect_failure \
    'a direct pilotage-xplane-trial runtime dependency' \
    'runtime dependency pilotage-xplane-trial'
write_clean_manifests

printf '%s\n' \
    '[package]' \
    'name = "flight-tune"' \
    'version = "0.0.0"' \
    'edition = "2021"' \
    '[dependencies.trial]' \
    'package = "pilotage-xplane-trial"' \
    'path = "../../adapters/pilotage-xplane-trial"' \
    > "$fixture/tools/flight-tune/Cargo.toml"
expect_failure \
    'an aliased pilotage-xplane-trial runtime dependency' \
    'runtime dependency pilotage-xplane-trial'
write_clean_manifests

printf '%s\n' \
    '[workspace]' \
    'members = [' \
    '    "adapters/flight-tune-xplane",' \
        '    "adapters/pilotage-xplane-trial",' \
        '    "crates/pilotage-trial",' \
        '    "injected",' \
        '    "tools/flight-tune",' \
        '    "tools/flight-tune-aviate",' \
        '    "tools/flight-tune-campaign",' \
        '    "vendor/errno",' \
        '    "vendor/libproc",' \
        '    "vendor/sysctl",' \
    ']' \
    'resolver = "2"' \
    '[workspace.dependencies]' \
    'adapter = { package = "flight-tune-xplane", path = "adapters/flight-tune-xplane" }' \
    > "$fixture/Cargo.toml"
printf '%s\n' \
    '[package]' \
    'name = "flight-tune"' \
    'version = "0.0.0"' \
    'edition = "2021"' \
    '[dependencies]' \
    'adapter = { workspace = true }' \
    > "$fixture/tools/flight-tune/Cargo.toml"
expect_failure \
    'an inherited workspace dependency alias' \
    'runtime dependency flight-tune-xplane'
write_workspace
write_clean_manifests

printf '%s\n' \
    '[package]' \
    'name = "flight-tune"' \
    'version = "0.0.0"' \
    'edition = "2021"' \
    '[build-dependencies]' \
    'adapter = { package = "flight-tune-xplane", path = "../../adapters/flight-tune-xplane" }' \
    > "$fixture/tools/flight-tune/Cargo.toml"
expect_failure \
    'a flight-tune-xplane build dependency' \
    'runtime dependency flight-tune-xplane'
write_clean_manifests

printf '%s\n' \
    '[package]' \
    'name = "flight-tune"' \
    'version = "0.0.0"' \
    'edition = "2021"' \
    '[dependencies]' \
    '[dev-dependencies]' \
    'pilotage-xplane-trial = { path = "../../adapters/pilotage-xplane-trial" }' \
    > "$fixture/tools/flight-tune/Cargo.toml"
run_guard >/dev/null
write_clean_manifests

printf '%s\n' \
    '[package]' \
    'name = "flight-tune"' \
    'version = "0.0.0"' \
    'edition = "2021"' \
    '[dependencies]' \
    'injected = { path = "../../injected" }' \
    > "$fixture/tools/flight-tune/Cargo.toml"
expect_failure \
    'a new direct production dependency' \
    'has unreviewed direct production dependency injected'
write_clean_manifests

printf '%s\n' \
    'pub struct SharedContract {' \
    '    pub xplane: String,' \
    '    pub acf: String,' \
    '    pub aircraft_file: String,' \
    '    pub trial_plugin: String,' \
    '    pub bridge_plugin: String,' \
    '    pub weather_plugin: String,' \
    '    pub host_application_id: String,' \
    '    pub xplane_version: String,' \
    '    pub sdk_version: String,' \
    '}' \
    > "$fixture/crates/pilotage-trial/src/lib.rs"
expect_failure_with_all \
    'all forbidden shared field identifiers' \
    'simulator-specific field xplane' \
    'simulator-specific field acf' \
    'simulator-specific field aircraft_file' \
    'simulator-specific field trial_plugin' \
    'simulator-specific field bridge_plugin' \
    'simulator-specific field weather_plugin' \
    'simulator-specific field host_application_id' \
    'simulator-specific field xplane_version' \
    'simulator-specific field sdk_version'
write_shared_contract

printf '%s\n' \
    'pub struct SharedContract<T>' \
    'where' \
    '    T: Clone,' \
    '{' \
    '    pub candidate_xplane_version_digest: T,' \
    '}' \
    > "$fixture/crates/pilotage-trial/src/lib.rs"
expect_failure \
    'a generic shared struct with a later body brace' \
    'simulator-specific field candidate_xplane_version_digest'
write_shared_contract

printf '%s\n' \
    'pub struct SharedContract {' \
    '    // }' \
    '    pub candidate_xplane_version_digest: String,' \
    '}' \
    > "$fixture/crates/pilotage-trial/src/lib.rs"
expect_failure \
    'a shared field after a comment brace' \
    'simulator-specific field candidate_xplane_version_digest'
write_shared_contract

printf '%s\n' \
    'pub enum SharedContract {' \
    '    Neutral,' \
    '    XPlaneReplay(String),' \
    '}' \
    > "$fixture/crates/pilotage-trial/src/lib.rs"
expect_failure \
    'a simulator name inside a shared enum variant' \
    'simulator-specific variant XPlaneReplay'
write_shared_contract

printf '%s\n' \
    'pub enum SharedContract {' \
    '    Neutral = 1 << 0,' \
    '    XPlaneReplay = 2,' \
    '}' \
    > "$fixture/crates/pilotage-trial/src/lib.rs"
expect_failure \
    'a simulator variant after a shift expression' \
    'simulator-specific variant XPlaneReplay'
write_shared_contract

printf '%s\n' \
    'pub struct SharedContract {' \
    '    pub mask: [u8; 1 << 2],' \
    '    pub xplane_new: String,' \
    '}' \
    > "$fixture/crates/pilotage-trial/src/lib.rs"
expect_failure \
    'a simulator field after a shift expression' \
    'simulator-specific field xplane_new'
write_shared_contract

printf '%s\n' \
    'macro_rules! define_contract {' \
    "    (\$field:ident) => {" \
    "        pub struct SharedContract { pub \$field: String }" \
    '    };' \
    '}' \
    'define_contract!(xplane_version);' \
    > "$fixture/crates/pilotage-trial/src/lib.rs"
expect_failure \
    'a macro-generated simulator field' \
    'has a new simulator-specific token xplane'
write_shared_contract

printf '%s\n' \
    'pub struct SharedContract {' \
    '    #[serde(rename = "x\u{70}lane_new")]' \
    '    pub id: String,' \
    '}' \
    > "$fixture/crates/pilotage-trial/src/lib.rs"
expect_failure_with_all \
    'an escaped simulator-specific serialized field name' \
    'has a new simulator-specific token xplane' \
    'has a simulator-specific Serde name xplane'
write_shared_contract

printf '%s\n' \
    'pub struct SharedContract { pub id: String }' \
    'impl serde::Serialize for SharedContract {' \
    '    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>' \
    '    where S: serde::Serializer {' \
    '        let key = concat!("x", "plane");' \
    '        serializer.serialize_str(key)' \
    '    }' \
    '}' \
    > "$fixture/crates/pilotage-trial/src/lib.rs"
expect_failure \
    'a hand-written shared serializer' \
    'has an unreviewed manual Serde impl'
write_shared_contract

printf '%s\n' \
    'use serde::Serialize as WireFormat;' \
    'pub struct SharedContract { pub id: String }' \
    'impl WireFormat for SharedContract {' \
    '    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>' \
    '    where S: serde::Serializer {' \
    '        serializer.serialize_str("neutral")' \
    '    }' \
    '}' \
    > "$fixture/crates/pilotage-trial/src/lib.rs"
expect_failure_with_all \
    'an aliased hand-written shared serializer' \
    'aliases a protected derive import' \
    'has an unreviewed manual Serde impl'
write_shared_contract

printf '%s\n' \
    'use injected::Serialize;' \
    '#[derive(Serialize)]' \
    'pub struct SharedContract { pub id: String }' \
    > "$fixture/crates/pilotage-trial/src/lib.rs"
expect_failure \
    'a noncanonical protected derive import' \
    'imports protected derive Serialize from a noncanonical crate'
write_shared_contract

printf '%s\n' \
    '#[path = "../../../outside-contract.rs"]' \
    'mod outside_contract;' \
    > "$fixture/crates/pilotage-trial/src/lib.rs"
printf '%s\n' \
    'pub struct SharedContract { pub xplane_version: String }' \
    > "$fixture/outside-contract.rs"
expect_failure \
    'a production path outside the shared source root' \
    'path attribute leaves its source root'
rm "$fixture/outside-contract.rs"
write_shared_contract

printf '%s\n' \
    '#[cfg_attr(not(test), path = "../../../outside-contract.rs")]' \
    'mod outside_contract;' \
    > "$fixture/crates/pilotage-trial/src/lib.rs"
expect_failure \
    'a conditional production module path' \
    'has a conditional module path'
write_shared_contract

printf '%s\n' \
    '#[path = "hidden.txt"]' \
    'mod hidden;' \
    > "$fixture/crates/pilotage-trial/src/lib.rs"
printf '%s\n' \
    'pub struct SharedContract { pub xplane_version: String }' \
    > "$fixture/crates/pilotage-trial/src/hidden.txt"
expect_failure \
    'a production module with a non-Rust extension' \
    'has a non-Rust module path'
rm "$fixture/crates/pilotage-trial/src/hidden.txt"
write_shared_contract

printf '%s\n' \
    'include!(concat!(env!("OUT_DIR"), "/unreviewed.rs"));' \
    > "$fixture/tools/flight-tune/src/lib.rs"
expect_failure \
    'an unreviewed generated source include' \
    'has an unreviewed generated Rust include'
write_shared_contract

printf '%s\n' \
    'include!{concat!(env!("OUT_DIR"), "/unreviewed.rs")};' \
    > "$fixture/tools/flight-tune/src/lib.rs"
expect_failure \
    'an unreviewed brace-delimited generated source include' \
    'has an unreviewed generated Rust include'
write_shared_contract

printf '%s\n' \
    'include![concat!(env!("OUT_DIR"), "/unreviewed.rs")];' \
    > "$fixture/tools/flight-tune/src/lib.rs"
expect_failure \
    'an unreviewed bracket-delimited generated source include' \
    'has an unreviewed generated Rust include'
write_shared_contract

mkdir -p \
    "$fixture/tools/flight-tune/src/flight_quality" \
    "$fixture/tools/flight-tune/build_support"
printf '%s\n' \
    'include!(concat!(env!("OUT_DIR"), "/evaluator_source_identity.rs"));' \
    > "$fixture/tools/flight-tune/src/flight_quality/identity.rs"
printf '%s\n' \
    '#[path = "build_support/evaluator_source_identity.rs"]' \
    'mod evaluator_source_identity;' \
    'fn main() {}' \
    > "$fixture/tools/flight-tune/build.rs"
printf '%s\n' 'pub fn generate() {}' \
    > "$fixture/tools/flight-tune/build_support/evaluator_source_identity.rs"

printf '%s\n' \
    '[package]' \
    'name = "flight-tune"' \
    'version = "0.0.0"' \
    'edition = "2021"' \
    'build = "malicious_build.rs"' \
    '[dependencies]' \
    > "$fixture/tools/flight-tune/Cargo.toml"
printf '%s\n' 'fn main() {}' \
    > "$fixture/tools/flight-tune/malicious_build.rs"
expect_failure \
    'an alternate generated-source build target' \
    'has an unreviewed custom build target'
rm "$fixture/tools/flight-tune/malicious_build.rs"
write_clean_manifests

printf '%s\n' \
    '[package]' \
    'name = "pilotage-trial"' \
    'version = "0.0.0"' \
    'edition = "2021"' \
    'build = "build.rs"' \
    '[dependencies]' \
    > "$fixture/crates/pilotage-trial/Cargo.toml"
printf '%s\n' 'fn main() {}' \
    > "$fixture/crates/pilotage-trial/build.rs"
expect_failure \
    'a custom build target in a shared contract package' \
    'has an unreviewed custom build target'
rm "$fixture/crates/pilotage-trial/build.rs"
write_clean_manifests

printf '%s\n' \
    '#![cfg(any())]' \
    'include!(concat!(env!("OUT_DIR"), "/evaluator_source_identity.rs"));' \
    > "$fixture/tools/flight-tune/src/flight_quality/identity.rs"
expect_failure \
    'a file-disabled allowlisted include' \
    'has an attributed generated source file'

printf '%s\n' \
    '#[cfg(any())]' \
    'include!(concat!(env!("OUT_DIR"), "/evaluator_source_identity.rs"));' \
    > "$fixture/tools/flight-tune/src/flight_quality/identity.rs"
expect_failure \
    'a conditionally disabled allowlisted include' \
    'has an unreviewed item macro include'

printf '%s\n' \
    'include!(concat!(env!("OUT_DIR"), "/evaluator_source_identity.rs"));' \
    > "$fixture/tools/flight-tune/src/flight_quality/identity.rs"
expect_failure \
    'an unfrozen allowlisted include generator' \
    'generated source input has an unreviewed digest'

printf '%s\n' \
    'include!(concat!(env!("OUT_DIR"), "/evaluator_source_identity.rs"));' \
    'include!(concat!(env!("OUT_DIR"), "/evaluator_source_identity.rs"));' \
    > "$fixture/tools/flight-tune/src/flight_quality/identity.rs"
expect_failure \
    'a repeated allowlisted generated include' \
    'repeats its reviewed generated Rust include'

printf '%s\n' \
    'include!(concat!(env!("OUT_DIR"), "/evaluator_source_identity.rs"));' \
    > "$fixture/tools/flight-tune/src/flight_quality/identity.rs"

printf '%s\n' 'pub const IDENTITY: &str = "manual";' \
    > "$fixture/tools/flight-tune/src/flight_quality/identity.rs"
expect_failure \
    'an allowlisted identity file without its generated include' \
    'omits its reviewed generated Rust include'

printf '%s\n' \
    'include!(concat!(env!("OUT_DIR"), "/evaluator_source_identity.rs"));' \
    > "$fixture/tools/flight-tune/src/flight_quality/identity.rs"

printf '%s\n' \
    'fn hidden_identity() {' \
    '    include!(concat!(env!("OUT_DIR"), "/evaluator_source_identity.rs"));' \
    '}' \
    > "$fixture/tools/flight-tune/src/flight_quality/identity.rs"
expect_failure \
    'an allowlisted generated include inside a function block' \
    'has an unreviewed generated Rust include'

printf '%s\n' \
    'include!(concat!(env!("OUT_DIR"), "/evaluator_source_identity.rs"));' \
    > "$fixture/tools/flight-tune/src/flight_quality/identity.rs"

printf '%s\n' \
    'mod payload;' \
    'fn main() {}' \
    > "$fixture/tools/flight-tune/build.rs"
printf '%s\n' \
    'const GENERATED: &str = "pub struct Shared { pub xplane_version: String }";' \
    > "$fixture/tools/flight-tune/payload.rs"
expect_failure \
    'an allowlisted generator with an unfrozen payload route' \
    'generated source input has an unreviewed digest'
rm "$fixture/tools/flight-tune/payload.rs"
rm "$fixture/tools/flight-tune/src/flight_quality/identity.rs"
rm "$fixture/tools/flight-tune/build_support/evaluator_source_identity.rs"
rm "$fixture/tools/flight-tune/build.rs"

mkdir -p "$fixture/shared-symlink-target"
printf '%s\n' \
    'pub struct SharedContract {' \
    '    pub xplane_new: String,' \
    '}' \
    > "$fixture/shared-symlink-target/contract.rs"
ln -s "$fixture/shared-symlink-target" \
    "$fixture/crates/pilotage-trial/src/contracts"
expect_failure \
    'a neutral source directory symlink' \
    'production source path is a symlink'
rm "$fixture/crates/pilotage-trial/src/contracts"

mkdir -p "$fixture/campaign-symlink-target"
printf '%s\n' \
    'pub struct CampaignConfig {' \
    '    pub xplane_new: String,' \
    '}' \
    > "$fixture/campaign-symlink-target/contract.rs"
ln -s "$fixture/campaign-symlink-target" \
    "$fixture/tools/flight-tune-campaign/src/config/contracts"
expect_failure \
    'a campaign config source directory symlink' \
    'production source path is a symlink'
rm "$fixture/tools/flight-tune-campaign/src/config/contracts"

printf '%s\n' \
    'pub enum SharedContract {' \
    '    Neutral { xplane_version: String },' \
    '}' \
    > "$fixture/crates/pilotage-trial/src/lib.rs"
expect_failure \
    'a simulator name inside a shared enum field' \
    'simulator-specific field xplane_version'
write_shared_contract

printf '%s\n' \
    'pub struct CampaignConfig {' \
    '    pub xplane: XPlaneCampaignConfig,' \
    '    pub aviate_xplane_contract: PinnedFile,' \
    '}' \
    'pub struct XPlaneCampaignConfig {' \
    '    pub weather_plugin_digest: String,' \
    '}' \
    > "$fixture/tools/flight-tune-campaign/src/config.rs"
run_guard >/dev/null

printf '%s\n' \
    'pub struct CampaignConfig {' \
    '    #[serde(flatten)]' \
    '    pub xplane: XPlaneCampaignConfig,' \
    '    pub aviate_xplane_contract: PinnedFile,' \
    '}' \
    'pub struct XPlaneCampaignConfig {' \
    '    pub weather_plugin_digest: String,' \
    '}' \
    > "$fixture/tools/flight-tune-campaign/src/config.rs"
expect_failure \
    'a flattened campaign adapter field' \
    'has an unreviewed Serde option flatten'

printf '%s\n' \
    '#[derive(Serialize)]' \
    '#[serde(transparent)]' \
    'pub struct CampaignWrapper(pub XPlaneCampaignConfig);' \
    > "$fixture/tools/flight-tune-campaign/src/config/wrapper_escape.rs"
expect_failure \
    'a transparent campaign adapter wrapper' \
    'has an unreviewed Serde option transparent'
rm "$fixture/tools/flight-tune-campaign/src/config/wrapper_escape.rs"

printf '%s\n' \
    'pub struct SearchGroupConfig {' \
    '    pub backend: XPlaneCampaignConfig,' \
    '}' \
    > "$fixture/tools/flight-tune-campaign/src/config/field_type_escape.rs"
expect_failure \
    'a neutral campaign field with an adapter type' \
    'SearchGroupConfig has simulator-specific field_type:backend XPlaneCampaignConfig'
rm "$fixture/tools/flight-tune-campaign/src/config/field_type_escape.rs"

printf '%s\n' \
    'pub struct SearchGroupConfig(pub XPlaneCampaignConfig);' \
    > "$fixture/tools/flight-tune-campaign/src/config/newtype_escape.rs"
expect_failure \
    'a campaign newtype with an adapter type' \
    'SearchGroupConfig has simulator-specific tuple_field_type:0 XPlaneCampaignConfig'
rm "$fixture/tools/flight-tune-campaign/src/config/newtype_escape.rs"

printf '%s\n' \
    'use serde::{Deserialize, Serialize};' \
    '#[derive(Serialize, Deserialize)]' \
    'pub struct SharedCampaignContract(pub XPlaneCampaignConfig);' \
    > "$fixture/tools/flight-tune-campaign/src/newtype_escape.rs"
expect_failure \
    'a serialized campaign newtype outside config' \
    'SharedCampaignContract has simulator-specific tuple_field_type:0 XPlaneCampaignConfig'
rm "$fixture/tools/flight-tune-campaign/src/newtype_escape.rs"

printf '%s\n' \
    'pub struct SearchGroupConfig<Backend = XPlaneCampaignConfig> {' \
    '    pub backend: Backend,' \
    '}' \
    > "$fixture/tools/flight-tune-campaign/src/config/header_escape.rs"
expect_failure \
    'a campaign generic default with an adapter type' \
    'SearchGroupConfig has simulator-specific header_type XPlaneCampaignConfig'
rm "$fixture/tools/flight-tune-campaign/src/config/header_escape.rs"

printf '%s\n' \
    'pub struct SearchGroupConfig<Backend>(pub Backend)' \
    'where' \
    '    Backend: XPlaneBackend;' \
    > "$fixture/tools/flight-tune-campaign/src/config/tuple_tail_escape.rs"
expect_failure \
    'a campaign tuple tail with an adapter bound' \
    'SearchGroupConfig has simulator-specific header_type XPlaneBackend'
rm "$fixture/tools/flight-tune-campaign/src/config/tuple_tail_escape.rs"

printf '%s\n' \
    'type BackendConfig = XPlaneCampaignConfig;' \
    'pub struct SearchGroupConfig { pub backend: BackendConfig }' \
    > "$fixture/tools/flight-tune-campaign/src/config/alias_escape.rs"
expect_failure \
    'a campaign adapter type alias' \
    'has unreviewed simulator type alias BackendConfig for XPlaneCampaignConfig'
rm "$fixture/tools/flight-tune-campaign/src/config/alias_escape.rs"

printf '%s\n' \
    'use crate::XPlaneCampaignConfig as BackendConfig;' \
    'pub struct SearchGroupConfig { pub backend: BackendConfig }' \
    > "$fixture/tools/flight-tune-campaign/src/config/import_alias_escape.rs"
expect_failure \
    'a campaign adapter import alias' \
    'has a simulator-specific import alias'
rm "$fixture/tools/flight-tune-campaign/src/config/import_alias_escape.rs"

printf '%s\n' \
    'pub struct CampaignConfig {' \
    '    pub xplane: XPlaneCampaignConfig,' \
    '    pub aviate_xplane_contract: PinnedFile,' \
    '    pub xplane_new: XPlaneCampaignConfig,' \
    '}' \
    'pub struct XPlaneCampaignConfig {' \
    '    pub weather_plugin_digest: String,' \
    '}' \
    > "$fixture/tools/flight-tune-campaign/src/config.rs"
expect_failure \
    'a new CampaignConfig X-Plane field' \
    'CampaignConfig has simulator-specific field xplane_new'
write_campaign_contract

printf '%s\n' \
    '#[path = "escape.rs"]' \
    'mod escape;' \
    > "$fixture/tools/flight-tune-campaign/src/config.rs"
printf '%s\n' \
    'pub struct CampaignConfig { pub xplane_new: String }' \
    > "$fixture/tools/flight-tune-campaign/src/escape.rs"
expect_failure \
    'a campaign sibling shared field' \
    'CampaignConfig has simulator-specific field xplane_new'
rm "$fixture/tools/flight-tune-campaign/src/escape.rs"
write_campaign_contract

printf '%s\n' \
    'macro_rules! define_contract {' \
    "    (\$field:ident) => {" \
    "        pub struct CampaignConfig { pub \$field: String }" \
    '    };' \
    '}' \
    'define_contract!(xplane_new);' \
    > "$fixture/tools/flight-tune-campaign/src/config/macro_escape.rs"
expect_failure_with_all \
    'a macro-generated campaign field' \
    'has a production macro definition' \
    'has an unreviewed item macro define_contract'
rm "$fixture/tools/flight-tune-campaign/src/config/macro_escape.rs"

printf '%s\n' \
    '#[inject_contract]' \
    'pub struct CampaignConfig { pub id: String }' \
    > "$fixture/tools/flight-tune-campaign/src/config/attribute_escape.rs"
expect_failure \
    'a campaign procedural attribute contract' \
    'has an unreviewed contract attribute inject_contract'
rm "$fixture/tools/flight-tune-campaign/src/config/attribute_escape.rs"

printf '%s\n' \
    '#[derive(InjectContract)]' \
    'pub struct CampaignConfig { pub id: String }' \
    > "$fixture/tools/flight-tune-campaign/src/config/derive_escape.rs"
expect_failure \
    'a campaign procedural derive contract' \
    'has an unreviewed derive macro InjectContract'
rm "$fixture/tools/flight-tune-campaign/src/config/derive_escape.rs"

printf '%s\n' \
    'pub struct XPlaneCampaignConfig {' \
    '    pub xplane_new: String,' \
    '}' \
    > "$fixture/tools/flight-tune-campaign/src/config/adapter_escape.rs"
expect_failure_with_all \
    'an adapter type name outside its exact file' \
    'unclassified public campaign contract XPlaneCampaignConfig' \
    'XPlaneCampaignConfig has simulator-specific field xplane_new'
rm "$fixture/tools/flight-tune-campaign/src/config/adapter_escape.rs"

printf '%s\n' \
    '#[derive(Serialize)]' \
    'pub struct SharedCampaignContract {' \
    '    pub id: String,' \
    '}' \
    > "$fixture/tools/flight-tune-campaign/src/config.rs"
expect_failure \
    'an unclassified public campaign contract' \
    'unclassified public campaign contract SharedCampaignContract'
write_campaign_contract

printf '%s\n' \
    'mod shared;' \
    'pub use shared::SharedCampaignContract;' \
    > "$fixture/tools/flight-tune-campaign/src/config.rs"
printf '%s\n' \
    '#[derive(Serialize)]' \
    'pub struct SharedCampaignContract {' \
    '    pub xplane_version: String,' \
    '}' \
    > "$fixture/tools/flight-tune-campaign/src/config/shared.rs"
expect_failure_with_all \
    'an unclassified campaign submodule contract' \
    'unclassified public campaign contract SharedCampaignContract' \
    'simulator-specific field xplane_version'
rm "$fixture/tools/flight-tune-campaign/src/config/shared.rs"
write_campaign_contract

printf '%s\n' \
    'pub use crate::config::XPlaneCampaignConfig;' \
    > "$fixture/tools/flight-tune-campaign/src/reexport_escape.rs"
expect_failure \
    'a simulator-specific public use re-export' \
    'has a simulator-specific public re-export'
rm "$fixture/tools/flight-tune-campaign/src/reexport_escape.rs"

printf '%s\n' \
    'pub extern crate flight_tune_xplane as neutral_adapter;' \
    > "$fixture/tools/flight-tune-campaign/src/reexport_escape.rs"
expect_failure \
    'a simulator-specific public extern crate re-export' \
    'has a simulator-specific public re-export'
rm "$fixture/tools/flight-tune-campaign/src/reexport_escape.rs"

printf '%s\n' \
    'extern crate flight_tune_xplane as neutral_adapter;' \
    > "$fixture/tools/flight-tune-campaign/src/reexport_escape.rs"
expect_failure \
    'a simulator-specific private extern crate alias' \
    'has a simulator-specific extern crate'
rm "$fixture/tools/flight-tune-campaign/src/reexport_escape.rs"

run_guard >/dev/null
echo "flight tuning boundary guard self-test: OK"
