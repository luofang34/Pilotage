use super::*;
use crate::{NavigationPoint, NavigationSource};

fn result(authority: &str, latitude: f64, start: i64, end: i64) -> SearchResult {
    SearchResult {
        point: NavigationPoint {
            key: authority.into(),
            identifier: "SAME".into(),
            kind: "waypoint".into(),
            name: "Published point".into(),
            region: authority.into(),
            latitude_deg: latitude,
            longitude_deg: 0.0,
        },
        source: NavigationSource {
            release_id: authority.into(),
            authority: authority.into(),
            edition: start.to_string(),
            source_digest: "a".repeat(64),
            effective_at: start,
            expires_at: end,
        },
    }
}

#[test]
fn current_records_replace_expired_duplicates_but_not_other_places() {
    let merged = merge_navigation_matches(
        vec![
            result("expired", 40.0, 1, 10),
            result("nasr", 40.0, 10, 20),
            result("cifp", 40.00001, 10, 20),
            result("foreign", 50.0, 10, 20),
            result("upcoming", 60.0, 20, 30),
        ],
        "SAME",
        15,
        100,
    );
    assert_eq!(merged.len(), 2);
    assert!(merged.iter().all(|item| item.source.expires_at > 15));
    assert!(merged.iter().any(|item| item.point.latitude_deg == 50.0));
}

#[test]
fn expired_data_is_available_until_a_current_record_exists() {
    let merged = merge_navigation_matches(vec![result("expired", 40.0, 1, 10)], "SAME", 10, 100);
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0].source.authority, "expired");
}

#[test]
fn different_point_types_at_one_position_remain_separate() {
    let point = result("nasr", 40.0, 10, 20);
    let mut airport = point.clone();
    airport.point.kind = "airport".into();
    assert_eq!(
        merge_navigation_matches(vec![point, airport], "SAME", 15, 100).len(),
        2
    );
}
