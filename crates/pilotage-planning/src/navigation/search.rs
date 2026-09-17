//! Search result selection across source editions.

use super::SearchResult;
use std::cmp::Reverse;

#[cfg(test)]
mod tests;

/// Removes duplicate source records while retaining equal identifiers at different places.
pub fn merge_navigation_matches(
    mut matches: Vec<SearchResult>,
    query: &str,
    now: i64,
    limit: u32,
) -> Vec<SearchResult> {
    matches.retain(|item| item.source.effective_at <= now);
    matches.sort_by_key(|item| {
        (
            item.source.expires_at <= now,
            Reverse(item.source.effective_at),
            item.point.name.is_empty(),
            item.source.authority.clone(),
            item.source.release_id.clone(),
            item.point.key.clone(),
        )
    });
    let mut unique: Vec<SearchResult> = Vec::new();
    for item in matches {
        if !unique.iter().any(|other| same_place(&item, other)) {
            unique.push(item);
        }
    }
    unique.sort_by_key(|item| {
        (
            !item.point.identifier.eq_ignore_ascii_case(query.trim()),
            item.source.expires_at <= now,
            item.point.identifier.clone(),
            item.point.region.clone(),
            item.source.release_id.clone(),
            item.point.key.clone(),
        )
    });
    unique.truncate(limit.min(100) as usize);
    unique
}

fn same_place(a: &SearchResult, b: &SearchResult) -> bool {
    if !a.point.identifier.eq_ignore_ascii_case(&b.point.identifier) || a.point.kind != b.point.kind
    {
        return false;
    }
    let lat_a = a.point.latitude_deg.to_radians();
    let lat_b = b.point.latitude_deg.to_radians();
    let longitude = (a.point.longitude_deg - b.point.longitude_deg).to_radians();
    let haversine = ((lat_a - lat_b) / 2.0).sin().powi(2)
        + lat_a.cos() * lat_b.cos() * (longitude / 2.0).sin().powi(2);
    // NASR and CIFP can round the same published position differently.
    2.0 * haversine.clamp(0.0, 1.0).sqrt().asin() * 6_371_008.8 <= 185.2
}
