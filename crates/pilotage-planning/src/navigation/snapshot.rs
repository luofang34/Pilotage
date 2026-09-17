//! FAA snapshot conversion at the navigation-source boundary.

use super::{NavigationDataset, NavigationPoint, NavigationSource};
use crate::PlanningError;
use aerocontext_core::NavPointKind;
use sha2::{Digest, Sha256};

impl NavigationDataset {
    /// Converts an encoded NASR or CIFP snapshot into source-neutral search records.
    pub fn from_acnav(release_id: String, bytes: &[u8]) -> Result<Self, PlanningError> {
        let snapshot = aerocontext_navdata::decode(bytes)?;
        let source = NavigationSource {
            release_id,
            authority: snapshot.cycle.authority.slug().into(),
            edition: snapshot.cycle.effective_on.to_string(),
            source_digest: format!("{:x}", Sha256::digest(bytes)),
            effective_at: snapshot
                .cycle
                .effective_on
                .and_time(chrono_midnight())
                .and_utc()
                .timestamp(),
            expires_at: snapshot
                .cycle
                .next_effective_on
                .and_time(chrono_midnight())
                .and_utc()
                .timestamp(),
        };
        let mut records = std::collections::BTreeMap::new();
        for point in snapshot.points {
            let kind = match point.kind {
                NavPointKind::Airport => "airport",
                NavPointKind::Waypoint => "waypoint",
                NavPointKind::Navaid => "navaid",
                _ => continue,
            };
            let region = point.region.unwrap_or_default();
            let identity = serde_json::to_vec(&(kind, &region, &point.ident, &point.position))?;
            let key = format!("{:x}", Sha256::digest(identity));
            records.entry(key.clone()).or_insert(NavigationPoint {
                key,
                identifier: point.ident,
                kind: kind.into(),
                name: point.name.unwrap_or_default(),
                region,
                latitude_deg: point.position.lat,
                longitude_deg: point.position.lon,
            });
        }
        let dataset = Self {
            schema_version: 1,
            source,
            points: records.into_values().collect(),
        };
        dataset.validate()?;
        Ok(dataset)
    }
}

fn chrono_midnight() -> chrono::NaiveTime {
    chrono::NaiveTime::MIN
}
