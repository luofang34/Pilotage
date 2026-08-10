//! AirspaceView contract tests.

#![allow(clippy::expect_used, clippy::panic)]

mod resolution;

use aerocontext_core::{GeoPoint, NavDataCycle, NavDataSnapshot, NavPoint, NavPointKind};
use chrono::NaiveDate;

use crate::{
    AeronauticalUpdateV1, IdentifiedNavdataSnapshotV1, NavdataIdentityV1, SubjectExtentV1,
    SubjectFamilyV1, SubjectReferenceV1, navdata_cycle_id,
};

fn snapshot(effective_on: NaiveDate, points: Vec<NavPoint>) -> IdentifiedNavdataSnapshotV1 {
    let cycle = NavDataCycle::faa_nasr(effective_on).expect("test cycle must be valid");
    let snapshot = NavDataSnapshot::new(cycle, points);
    let identity = NavdataIdentityV1 {
        cycle: navdata_cycle_id(&snapshot),
        snapshot_id: format!("snapshot-{effective_on}"),
        snapshot_digest: format!("digest-{effective_on}"),
    };
    IdentifiedNavdataSnapshotV1::try_new(identity, snapshot)
        .expect("test identity must match the snapshot")
}

fn navaid(ident: &str, lat: f64) -> NavPoint {
    NavPoint::new(ident, NavPointKind::Navaid, GeoPoint { lat, lon: -71.0 })
}

fn subject(cycle: &str, family: SubjectFamilyV1, identifier: &str) -> SubjectReferenceV1 {
    SubjectReferenceV1 {
        cycle: cycle.to_owned(),
        family,
        identifier: identifier.to_owned(),
        parent_identifier: None,
        region: None,
        extent: SubjectExtentV1::Whole,
    }
}

fn update(subject: Option<SubjectReferenceV1>) -> AeronauticalUpdateV1 {
    AeronauticalUpdateV1 {
        update_id: "update-1".into(),
        display_text: "Facility status changed".into(),
        subject,
        geometry: None,
    }
}
