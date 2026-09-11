#![allow(clippy::expect_used)]

use aerocontext_core::{NavDataCycle, NavDataSnapshot};
use aerocontext_navdata::{Manifest, ManifestEntry};

use crate::{
    Channel, ContentDigest, Distribution, PackageStore, Product, fixtures::release,
    import_acnav_catalog,
};

#[test]
fn legacy_snapshot_uses_shared_installation_and_explicit_utc_activation() {
    let date = chrono::NaiveDate::from_ymd_opt(2026, 8, 6).expect("date");
    let cycle = NavDataCycle::faa_nasr(date).expect("cycle");
    let snapshot = NavDataSnapshot::new(cycle.clone(), Vec::new());
    let bytes = aerocontext_navdata::encode(&snapshot).expect("encode snapshot");
    let entry = ManifestEntry::new(
        &cycle,
        "2608",
        aerocontext_navdata::FORMAT_VERSION,
        "https://data.example.test/navigation.acnav",
        ContentDigest::of_bytes(&bytes).as_str(),
        bytes.len() as u64,
    )
    .with_no_redistribute(true);
    let generated = date.and_hms_opt(0, 0, 0).expect("time").and_utc();
    let manifest = Manifest::new(generated, vec![entry]);
    let coverage = release("coverage", b"fixture").coverage;
    let catalog = import_acnav_catalog(
        &manifest,
        &coverage,
        9 * 3600 + 60,
        Channel::Development,
        1,
        generated.timestamp() + 86_400,
    )
    .expect("import catalog");
    let package = &catalog.releases[0];
    assert_eq!(package.distribution, Distribution::Restricted);
    assert_eq!(
        package.validity.expect("validity").effective_at,
        generated.timestamp() + 32_460
    );
    assert!(
        catalog
            .current(
                Product::Navdata,
                "faa-nasr",
                &coverage.name,
                generated.timestamp(),
                Channel::Development
            )
            .is_none()
    );
    let directory = tempfile::tempdir().expect("directory");
    let mut store = PackageStore::open_blocking(directory.path()).expect("store");
    store
        .append_blocking(&package.artifacts[0], 0, &bytes)
        .expect("stage");
    let installed = store.finish_install_blocking(package).expect("install");
    let decoded = aerocontext_navdata::decode(
        &std::fs::read(installed.directory.join("navigation.acnav")).expect("installed bytes"),
    )
    .expect("decode");
    assert_eq!(decoded.cycle, cycle);
}

#[test]
fn future_legacy_format_and_invalid_activation_time_are_rejected() {
    let manifest: Manifest = serde_json::from_value(serde_json::json!({
        "manifest_version": 2, "generated_at": "2026-08-06T00:00:00Z", "entries": []
    }))
    .expect("manifest");
    let coverage = release("coverage", b"fixture").coverage;
    assert!(
        import_acnav_catalog(&manifest, &coverage, 0, Channel::Development, 0, i64::MAX).is_err()
    );
    let mut manifest = manifest;
    manifest.manifest_version = 1;
    assert!(
        import_acnav_catalog(
            &manifest,
            &coverage,
            86_400,
            Channel::Development,
            0,
            i64::MAX
        )
        .is_err()
    );
}
