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
    "$fixture/scripts" \
    "$fixture/tools/flight-tune/src" \
    "$fixture/tools/flight-tune-aviate/src/bin" \
    "$fixture/tools/flight-tune-aviate/src/support" \
    "$fixture/tools/flight-tune-aviate/src/tests" \
    "$fixture/tools/flight-tune-campaign/src/config"

write_workspace() {
    printf '%s\n' \
        '[workspace]' \
        'members = [' \
        '    "adapters/flight-tune-xplane",' \
        '    "adapters/pilotage-xplane-trial",' \
        '    "crates/pilotage-trial",' \
        '    "tools/flight-tune",' \
        '    "tools/flight-tune-aviate",' \
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
    printf '%s\n' \
        '[package]' \
        'name = "flight-tune-aviate"' \
        'version = "0.0.0"' \
        'edition = "2021"' \
        '[dependencies]' \
        'flight-tune-xplane = { path = "../../adapters/flight-tune-xplane" }' \
        'pilotage-xplane-trial = { path = "../../adapters/pilotage-xplane-trial" }' \
        > "$fixture/tools/flight-tune-aviate/Cargo.toml"
    printf '%s\n' 'pub fn host() {}' \
        > "$fixture/tools/flight-tune-aviate/src/lib.rs"
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
write_clean_manifests
write_allowlisted_import
write_shared_contract
write_campaign_contract
printf '%s\t%s\n' \
    'tools/flight-tune-aviate/src/driver.rs' \
    'useflight_tune_xplane::CausalJoinConfig;' \
    > "$fixture_allowlist"
run_guard >/dev/null

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
        '    "tools/flight-tune",' \
        '    "tools/flight-tune-aviate",' \
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

run_guard >/dev/null
echo "flight tuning boundary guard self-test: OK"
