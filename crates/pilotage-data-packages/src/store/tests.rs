#![allow(clippy::expect_used)]

use crate::{
    ContentDigest, PackageError, PackageStore, Selection,
    fixtures::{policy, release},
};

#[test]
fn local_import_replaces_partial_bytes_and_survives_reopen() {
    let directory = tempfile::tempdir().expect("store directory");
    let source = tempfile::tempdir().expect("source directory");
    let bytes: Vec<u8> = (0..262_147).map(|index| (index % 251) as u8).collect();
    let chart = release("local-chart", &bytes);
    let artifact = &chart.artifacts[0];
    std::fs::create_dir_all(source.path().join("symbols")).expect("source parent");
    std::fs::write(source.path().join(artifact.path.as_str()), &bytes).expect("source file");
    let mut store = PackageStore::open_blocking(directory.path()).expect("open");
    store
        .append_blocking(artifact, 0, b"invalid partial")
        .expect("interrupted transfer");
    let installed = store
        .import_directory_blocking(&chart, source.path(), artifact.bytes)
        .expect("local import");
    assert_eq!(
        std::fs::read(installed.directory.join(artifact.path.as_str())).expect("installed bytes"),
        bytes
    );
    drop(store);
    let store = PackageStore::open_blocking(directory.path()).expect("reopen");
    assert!(store.verify_installed_blocking(&chart.id).is_ok());
}

#[test]
fn corrupt_local_source_is_not_installed_and_can_be_retried() {
    let directory = tempfile::tempdir().expect("store directory");
    let source = tempfile::tempdir().expect("source directory");
    let chart = release("local-chart", b"valid");
    let artifact = &chart.artifacts[0];
    std::fs::create_dir_all(source.path().join("symbols")).expect("source parent");
    std::fs::write(source.path().join(artifact.path.as_str()), b"wrong").expect("corrupt source");
    let mut store = PackageStore::open_blocking(directory.path()).expect("open");
    assert!(matches!(
        store.import_directory_blocking(&chart, source.path(), 5),
        Err(PackageError::Digest { .. })
    ));
    assert!(
        store
            .installed_cached_blocking(&chart.id)
            .expect("catalog")
            .is_none()
    );
    std::fs::write(source.path().join(artifact.path.as_str()), b"valid").expect("repair source");
    store
        .import_directory_blocking(&chart, source.path(), 5)
        .expect("retry import");
    assert!(store.verify_installed_blocking(&chart.id).is_ok());
}

#[test]
fn interrupted_download_resumes_and_corruption_never_becomes_installed() {
    let directory = tempfile::tempdir().expect("store directory");
    let chart = release("chart", b"complete chart");
    let artifact = &chart.artifacts[0];
    let mut store = PackageStore::open_blocking(directory.path()).expect("open");
    store
        .append_blocking(artifact, 0, b"complete")
        .expect("first chunk");
    assert!(store.finish_install_blocking(&chart).is_err());
    assert!(
        store
            .installed_cached_blocking(&chart.id)
            .expect("read")
            .is_none()
    );
    drop(store);
    let mut store = PackageStore::open_blocking(directory.path()).expect("reopen");
    let plan = store.plan_blocking(&chart, 6).expect("resume");
    assert_eq!(plan.downloads[0].offset, 8);
    assert_eq!(plan.remaining_bytes, 6);
    assert!(store.append_blocking(artifact, 0, b"wrong").is_err());
    store
        .append_blocking(artifact, 8, b" ERROR")
        .expect("corrupt chunk");
    assert!(matches!(
        store.finish_install_blocking(&chart),
        Err(PackageError::Digest { .. })
    ));
    store
        .discard_download_blocking(&artifact.sha256)
        .expect("discard");
    store
        .append_blocking(artifact, 0, b"complete chart")
        .expect("download");
    let installed = store.finish_install_blocking(&chart).expect("install");
    assert_eq!(
        std::fs::read(installed.directory.join(artifact.path.as_str())).expect("resource"),
        b"complete chart"
    );
}

#[test]
fn active_data_survives_failed_update_and_pinned_session_cannot_change() {
    let directory = tempfile::tempdir().expect("directory");
    let mut store = PackageStore::open_blocking(directory.path()).expect("open");
    let current = release("current", b"current");
    store
        .append_blocking(&current.artifacts[0], 0, b"current")
        .expect("download");
    store.finish_install_blocking(&current).expect("install");
    let selection = Selection {
        name: "flight-1".to_owned(),
        root: current.id.clone(),
        pinned: true,
    };
    store
        .select_blocking(&selection, &policy())
        .expect("select");
    let next = release("next", b"next");
    assert!(matches!(
        store.plan_blocking(&next, 3),
        Err(PackageError::Space { .. })
    ));
    assert!(store.finish_install_blocking(&next).is_err());
    let change = Selection {
        root: next.id,
        ..selection.clone()
    };
    assert!(matches!(
        store.select_blocking(&change, &policy()),
        Err(PackageError::Retained { .. })
    ));
    assert!(store.remove_blocking(&current.id).is_err());
    drop(store);
    let store = PackageStore::open_blocking(directory.path()).expect("reopen");
    assert_eq!(
        store.selection_cached_blocking("flight-1").expect("read"),
        Some(selection)
    );
    assert!(store.verify_installed_blocking(&current.id).is_ok());
}

#[test]
fn dependency_and_renderer_admission_prevent_partial_display() {
    let directory = tempfile::tempdir().expect("directory");
    let mut store = PackageStore::open_blocking(directory.path()).expect("open");
    let resources = release("resources", b"resources");
    let mut chart = release("chart", b"chart");
    chart.dependencies.push(resources.id.clone());
    chart
        .renderer_capabilities
        .push("chart-paper-paths-v1".to_owned());
    store
        .append_blocking(&chart.artifacts[0], 0, b"chart")
        .expect("download");
    assert!(matches!(
        store.finish_install_blocking(&chart),
        Err(PackageError::Missing { .. })
    ));
    store
        .append_blocking(&resources.artifacts[0], 0, b"resources")
        .expect("download");
    store
        .finish_install_blocking(&resources)
        .expect("install resources");
    store
        .finish_install_blocking(&chart)
        .expect("install chart");
    let selection = Selection {
        name: "map".to_owned(),
        root: chart.id.clone(),
        pinned: false,
    };
    let mut policy = policy();
    assert!(store.select_blocking(&selection, &policy).is_err());
    policy
        .renderer_capabilities
        .insert("chart-paper-paths-v1".to_owned());
    store.select_blocking(&selection, &policy).expect("select");
    assert!(store.remove_blocking(&resources.id).is_err());
    policy.now = 200;
    assert!(store.select_blocking(&selection, &policy).is_err());
    assert_eq!(
        store.selection_cached_blocking("map").expect("read"),
        Some(selection)
    );
}

#[test]
fn immutable_identity_conflict_is_rejected_and_unreferenced_objects_are_reclaimed() {
    let directory = tempfile::tempdir().expect("directory");
    let mut store = PackageStore::open_blocking(directory.path()).expect("open");
    let chart = release("chart", b"chart");
    store
        .append_blocking(&chart.artifacts[0], 0, b"chart")
        .expect("download");
    store.finish_install_blocking(&chart).expect("install");
    let mut changed = chart.clone();
    changed.source_set = ContentDigest::of_bytes(b"correction");
    assert!(matches!(
        store.finish_install_blocking(&changed),
        Err(PackageError::IdentityConflict { .. })
    ));
    let orphan = directory
        .path()
        .join("objects")
        .join(ContentDigest::of_bytes(b"orphan").as_str());
    std::fs::write(orphan, b"orphan").expect("interrupted install object");
    assert_eq!(store.collect_unreferenced_blocking().expect("collect"), 6);
    assert!(store.verify_installed_blocking(&chart.id).is_ok());
    store.remove_blocking(&chart.id).expect("remove");
    assert_eq!(store.collect_unreferenced_blocking().expect("collect"), 5);
}

#[test]
fn failed_database_commit_recovers_without_exposing_an_install() {
    let directory = tempfile::tempdir().expect("directory");
    let mut store = PackageStore::open_blocking(directory.path()).expect("open");
    let chart = release("chart", b"chart");
    store
        .append_blocking(&chart.artifacts[0], 0, b"chart")
        .expect("download");
    store.connection.execute_batch(
        "CREATE TRIGGER fail_install BEFORE INSERT ON releases BEGIN SELECT RAISE(FAIL, 'test interruption'); END;"
    ).expect("failure injection");
    assert!(store.finish_install_blocking(&chart).is_err());
    assert!(
        store
            .installed_cached_blocking(&chart.id)
            .expect("read")
            .is_none()
    );
    assert!(
        directory
            .path()
            .join("releases/chart/symbols/atlas.bin")
            .exists()
    );
    drop(store);
    let mut store = PackageStore::open_blocking(directory.path()).expect("reopen");
    store
        .connection
        .execute_batch("DROP TRIGGER fail_install")
        .expect("recover writer");
    store
        .finish_install_blocking(&chart)
        .expect("resume install");
    assert!(store.verify_installed_blocking(&chart.id).is_ok());
}

#[test]
fn orphan_release_links_are_removed_before_objects_are_reclaimed() {
    let directory = tempfile::tempdir().expect("directory");
    let mut store = PackageStore::open_blocking(directory.path()).expect("open");
    let chart = release("chart", b"chart");
    store
        .append_blocking(&chart.artifacts[0], 0, b"chart")
        .expect("download");
    store.connection.execute_batch(
        "CREATE TRIGGER fail_install BEFORE INSERT ON releases BEGIN SELECT RAISE(FAIL, 'test interruption'); END;"
    ).expect("failure injection");
    assert!(store.finish_install_blocking(&chart).is_err());
    assert_eq!(store.collect_unreferenced_blocking().expect("reclaim"), 5);
    assert!(!directory.path().join("releases/chart").exists());
    assert_eq!(
        store
            .plan_blocking(&chart, 5)
            .expect("new plan")
            .remaining_bytes,
        5
    );
}

#[test]
fn a_second_writer_is_rejected_until_the_first_writer_closes() {
    let directory = tempfile::tempdir().expect("directory");
    let store = PackageStore::open_blocking(directory.path()).expect("open");
    assert!(matches!(
        PackageStore::open_blocking(directory.path()),
        Err(PackageError::StoreBusy { .. })
    ));
    drop(store);
    assert!(PackageStore::open_blocking(directory.path()).is_ok());
}

#[test]
fn terrain_dates_are_independent_but_navigation_joins_require_the_same_source() {
    let directory = tempfile::tempdir().expect("directory");
    let mut store = PackageStore::open_blocking(directory.path()).expect("open");
    let mut terrain = release("terrain", b"terrain");
    terrain.product = crate::Product::Terrain;
    terrain.validity = None;
    terrain.source_set = ContentDigest::of_bytes(b"terrain survey");
    store
        .append_blocking(&terrain.artifacts[0], 0, b"terrain")
        .expect("download");
    store
        .finish_install_blocking(&terrain)
        .expect("install terrain");
    let mut chart = release("chart", b"chart");
    chart.dependencies.push(terrain.id);
    store
        .append_blocking(&chart.artifacts[0], 0, b"chart")
        .expect("download");
    store
        .finish_install_blocking(&chart)
        .expect("install chart");
    let selection = Selection {
        name: "map".to_owned(),
        root: chart.id,
        pinned: false,
    };
    store
        .select_blocking(&selection, &policy())
        .expect("independent terrain edition");
    let mut nav = release("nav", b"nav");
    nav.product = crate::Product::Navdata;
    nav.source_set = ContentDigest::of_bytes(b"different navigation source");
    store
        .append_blocking(&nav.artifacts[0], 0, b"nav")
        .expect("download");
    store
        .finish_install_blocking(&nav)
        .expect("install navigation fixture");
    let mut other = release("other-chart", b"other");
    other.dependencies.push(nav.id);
    store
        .append_blocking(&other.artifacts[0], 0, b"other")
        .expect("download");
    store.finish_install_blocking(&other).expect("install");
    let other_selection = Selection {
        root: other.id,
        ..selection.clone()
    };
    assert!(store.select_blocking(&other_selection, &policy()).is_err());
    assert_eq!(
        store.selection_cached_blocking("map").expect("selected"),
        Some(selection)
    );
}
