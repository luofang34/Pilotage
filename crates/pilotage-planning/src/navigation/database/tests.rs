#![allow(clippy::expect_used, clippy::panic)]

use super::*;

fn dataset() -> NavigationDataset {
    NavigationDataset {
        schema_version: 1,
        source: NavigationSource {
            release_id: "france-2026-09".into(),
            authority: "sia-france".into(),
            edition: "2609".into(),
            source_digest: "a".repeat(64),
            effective_at: 1,
            expires_at: 100,
        },
        points: vec![
            point("one", "LFPG", "Paris Charles de Gaulle", "LF"),
            point("two", "FIX", "Saint-Étienne", "LF"),
            point("three", "FIX", "Different point", "EG"),
        ],
    }
}

fn point(key: &str, identifier: &str, name: &str, region: &str) -> NavigationPoint {
    NavigationPoint {
        key: key.into(),
        identifier: identifier.into(),
        kind: "airport".into(),
        name: name.into(),
        region: region.into(),
        latitude_deg: 49.0,
        longitude_deg: 2.0,
    }
}

#[test]
fn search_survives_reopen_and_keeps_ambiguous_source_records() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("nav.sqlite");
    drop(NavigationIndex::build_blocking(&path, &dataset()).expect("build"));
    let index = NavigationIndex::open_blocking(&path).expect("reopen");
    let matches = index.search_blocking("fix", 20).expect("search");
    assert_eq!(matches.len(), 2);
    assert_ne!(matches[0].point.key, matches[1].point.key);
    assert_eq!(matches[0].source.authority, "sia-france");
    assert_eq!(
        index.search_blocking("charles gau", 20).expect("name")[0]
            .point
            .identifier,
        "LFPG"
    );
    assert_eq!(
        index.search_blocking("etienne", 20).expect("accent")[0]
            .point
            .region,
        "LF"
    );
}

#[test]
fn query_syntax_is_literal_and_limits_are_bounded() {
    let directory = tempfile::tempdir().expect("directory");
    let index = NavigationIndex::build_blocking(&directory.path().join("nav.sqlite"), &dataset())
        .expect("build");
    assert!(
        index
            .search_blocking("\" OR *", 20)
            .expect("literal query")
            .is_empty()
    );
    assert!(index.search_blocking(" ", 20).expect("empty").is_empty());
    assert_eq!(index.search_blocking("FIX", 1).expect("limit").len(), 1);
    assert!(index.search_blocking(&"x".repeat(257), 20).is_err());
}

#[test]
fn invalid_import_cannot_replace_an_index() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("nav.sqlite");
    let input = dataset();
    NavigationIndex::build_blocking(&path, &input).expect("build");
    let mut invalid = input.clone();
    invalid.points[0].latitude_deg = f64::NAN;
    assert!(NavigationIndex::build_blocking(&path, &invalid).is_err());
    assert!(NavigationIndex::build_blocking(&path, &input).is_err());
    assert_eq!(
        NavigationIndex::open_blocking(&path)
            .expect("read")
            .source(),
        &input.source
    );
    invalid.points = vec![input.points[0].clone(), input.points[0].clone()];
    assert!(invalid.validate().is_err());
}

#[test]
fn incomplete_search_schema_is_rejected_when_opened() {
    let directory = tempfile::tempdir().expect("directory");
    let path = directory.path().join("nav.sqlite");
    drop(NavigationIndex::build_blocking(&path, &dataset()).expect("build"));
    Connection::open(&path)
        .expect("open")
        .execute("DROP TABLE points_search", [])
        .expect("remove search table");
    assert!(NavigationIndex::open_blocking(&path).is_err());
}
