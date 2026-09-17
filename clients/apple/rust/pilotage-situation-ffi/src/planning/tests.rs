#![allow(clippy::expect_used, clippy::panic)]
use super::*;
use aerocontext_core::{GeoPoint, NavDataCycle, NavDataSnapshot, NavPoint, NavPointKind};

#[test]
fn acnav_cache_keeps_exact_release_times_and_reopens_offline() {
    let directory = tempfile::tempdir().expect("directory");
    let request = fixture(directory.path());
    let cache = directory.path().join("cache");
    let session = NavigationSearchSession::new();
    session
        .replace_sources_blocking(vec![request.clone()], cache.to_string_lossy().into())
        .expect("load");
    let json = session
        .search_blocking("KTTN".into(), 20, 1_788_500_000)
        .expect("search");
    let matches: Vec<pilotage_planning::SearchResult> =
        serde_json::from_str(&json).expect("matches");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].source.effective_at, request.effective_at);
    assert_eq!(matches[0].source.expires_at, request.expires_at);
    let second = NavigationSearchSession::new();
    second
        .replace_sources_blocking(vec![request], cache.to_string_lossy().into())
        .expect("reopen");
    assert_eq!(
        second
            .search_blocking("KTTN".into(), 20, 1_788_500_000)
            .expect("cached search"),
        json
    );
}

#[test]
fn failed_source_replacement_keeps_the_complete_loaded_source_set() {
    let directory = tempfile::tempdir().expect("directory");
    let request = fixture(directory.path());
    let cache = directory.path().join("cache").to_string_lossy().to_string();
    let session = NavigationSearchSession::new();
    session
        .replace_sources_blocking(vec![request.clone()], cache.clone())
        .expect("load");
    let before = session
        .search_blocking("KTTN".into(), 20, 1_788_500_000)
        .expect("search");
    let mut wrong = request.clone();
    wrong.authority = "different-authority".into();
    assert!(
        session
            .replace_sources_blocking(vec![request, wrong], cache)
            .is_err()
    );
    assert_eq!(
        session
            .search_blocking("KTTN".into(), 20, 1_788_500_000)
            .expect("retained"),
        before
    );
}

fn fixture(root: &Path) -> NavigationIndexRequest {
    let date = chrono::NaiveDate::from_ymd_opt(2026, 9, 3).expect("date");
    let cycle = NavDataCycle::faa_nasr(date).expect("cycle");
    let start = cycle
        .effective_on
        .and_time(chrono::NaiveTime::MIN)
        .and_utc()
        .timestamp()
        + 32460;
    let end = cycle
        .next_effective_on
        .and_time(chrono::NaiveTime::MIN)
        .and_utc()
        .timestamp()
        + 32460;
    let snapshot = NavDataSnapshot::new(
        cycle,
        vec![NavPoint::new(
            "KTTN",
            NavPointKind::Airport,
            GeoPoint {
                lat: 40.2767,
                lon: -74.8135,
            },
        )],
    );
    let bytes = aerocontext_navdata::encode(&snapshot).expect("encode");
    let path = root.join("navigation.acnav");
    std::fs::write(&path, &bytes).expect("write");
    NavigationIndexRequest {
        release_id: "test-nasr".into(),
        authority: "faa-nasr".into(),
        path: path.to_string_lossy().into(),
        format: "acnav".into(),
        effective_at: start,
        expires_at: end,
        artifact_digest: format!("{:x}", Sha256::digest(bytes)),
    }
}
