//! Resolution behavior for all three subject cases.

use aerocontext_core::{GeoPoint, NavDataCycle, NavDataSnapshot, Runway};
use chrono::NaiveDate;

use super::{navaid, snapshot, subject, update};
use crate::{
    AeronauticalUpdateV1, AirspaceViewError, AirspaceViewV1, GeometryCoverageV1,
    GeometryResolutionV1, GeometryV1, IdentifiedNavdataSnapshotV1, MapCompletenessV1,
    NavdataIdentityV1, ResolutionFailureReasonV1, SubjectExtentV1, SubjectFamilyV1,
    UpdateGeometryV1, navdata_cycle_id,
};

#[test]
fn direct_geometry_skips_subject_resolution() {
    let snapshot = snapshot(date(2026, 1, 22), Vec::new());
    let mut update = update(Some(subject(
        "another-cycle",
        SubjectFamilyV1::Procedure,
        "IAP-1",
    )));
    update.geometry = Some(UpdateGeometryV1 {
        geometry: GeometryV1::Point {
            position: GeoPoint {
                lat: 42.0,
                lon: -71.0,
            },
        },
        coverage: GeometryCoverageV1::WholeSubject,
    });

    let result = AirspaceViewV1::derive(&snapshot, &[update]);

    assert!(result.updates[0].geometry.is_some());
    assert_eq!(result.updates[0].resolution, GeometryResolutionV1::Direct);
    assert_eq!(
        result.updates[0]
            .subject_identity
            .as_ref()
            .map(|identity| identity.cycle.as_str()),
        Some("another-cycle")
    );
}

#[test]
fn direct_geometry_cannot_enlarge_a_partial_subject() {
    let snapshot = snapshot(date(2026, 1, 22), Vec::new());
    let mut partial = subject(
        &snapshot.identity().cycle,
        SubjectFamilyV1::Runway,
        "04L/22R",
    );
    partial.extent = SubjectExtentV1::RunwaySegment {
        from_end: "04L".into(),
        start_offset_ft: 0,
        length_ft: 1_000,
    };
    let mut update = update(Some(partial));
    update.geometry = Some(UpdateGeometryV1 {
        geometry: GeometryV1::Point {
            position: GeoPoint {
                lat: 42.0,
                lon: -71.0,
            },
        },
        coverage: GeometryCoverageV1::WholeSubject,
    });

    let result = AirspaceViewV1::derive(&snapshot, &[update]);

    assert!(result.updates[0].geometry.is_none());
    assert!(matches!(
        result.updates[0].resolution,
        GeometryResolutionV1::Unresolved {
            reason: ResolutionFailureReasonV1::DirectGeometryExtentMismatch { .. }
        }
    ));
}

#[test]
fn unresolved_update_stays_in_the_result_without_geometry() {
    let snapshot = snapshot(date(2026, 1, 22), Vec::new());
    let update = update(Some(subject(
        &snapshot.identity().cycle,
        SubjectFamilyV1::Navaid,
        "MISSING",
    )));

    let result = AirspaceViewV1::derive(&snapshot, &[update]);

    assert_eq!(result.updates.len(), 1);
    assert!(result.updates[0].geometry.is_none());
    assert!(matches!(
        result.updates[0].resolution,
        GeometryResolutionV1::Unresolved {
            reason: ResolutionFailureReasonV1::UnknownIdentifier { .. }
        }
    ));
    assert_eq!(
        result.map_completeness,
        MapCompletenessV1::SupplementalOnly {
            updates_without_geometry: 1
        }
    );
}

#[test]
fn one_update_has_cycle_scoped_results() {
    let first = snapshot(date(2026, 1, 22), vec![navaid("BOS", 42.3)]);
    let second = snapshot(date(2026, 2, 19), vec![navaid("BOS", 42.4)]);
    let update = update(Some(subject(
        &first.identity().cycle,
        SubjectFamilyV1::Navaid,
        "BOS",
    )));

    let first_result = AirspaceViewV1::derive(&first, std::slice::from_ref(&update));
    let second_result = AirspaceViewV1::derive(&second, &[update]);

    assert_eq!(
        first_result.updates[0].resolution,
        GeometryResolutionV1::ResolvedFromNavdata
    );
    assert!(matches!(
        second_result.updates[0].resolution,
        GeometryResolutionV1::Unresolved {
            reason: ResolutionFailureReasonV1::IdentifierFromAnotherCycle { .. }
        }
    ));
}

#[test]
fn duplicate_identifier_is_typed_as_ambiguous() {
    let snapshot = snapshot(
        date(2026, 1, 22),
        vec![navaid("DUP", 42.0), navaid("DUP", 43.0)],
    );
    let update = update(Some(subject(
        &snapshot.identity().cycle,
        SubjectFamilyV1::Navaid,
        "DUP",
    )));

    let result = AirspaceViewV1::derive(&snapshot, &[update]);

    assert!(matches!(
        result.updates[0].resolution,
        GeometryResolutionV1::Unresolved {
            reason: ResolutionFailureReasonV1::AmbiguousMatch { matches: 2, .. }
        }
    ));
}

#[test]
fn unsupported_subject_family_is_typed_and_kept() {
    let snapshot = snapshot(date(2026, 1, 22), Vec::new());
    let update = update(Some(subject(
        &snapshot.identity().cycle,
        SubjectFamilyV1::Procedure,
        "RNAV-22",
    )));

    let result = AirspaceViewV1::derive(&snapshot, &[update]);

    assert!(matches!(
        result.updates[0].resolution,
        GeometryResolutionV1::Unresolved {
            reason: ResolutionFailureReasonV1::SubjectFamilyNotCarried {
                family: SubjectFamilyV1::Procedure
            }
        }
    ));
}

#[test]
fn partial_runway_closure_never_becomes_whole_runway_geometry() {
    let cycle = NavDataCycle::faa_nasr(date(2026, 1, 22)).expect("test cycle must be valid");
    let data =
        NavDataSnapshot::new(cycle, Vec::new()).with_runways(vec![Runway::new("KBOS", "04L/22R")]);
    let snapshot = identified(data);
    let mut runway = subject(
        &snapshot.identity().cycle,
        SubjectFamilyV1::Runway,
        "04L/22R",
    );
    runway.parent_identifier = Some("KBOS".into());
    runway.extent = SubjectExtentV1::RunwaySegment {
        from_end: "04L".into(),
        start_offset_ft: 0,
        length_ft: 1_000,
    };

    let result = AirspaceViewV1::derive(&snapshot, &[update(Some(runway))]);

    assert!(result.updates[0].geometry.is_none());
    assert!(matches!(
        result.updates[0].resolution,
        GeometryResolutionV1::Unresolved {
            reason: ResolutionFailureReasonV1::PartialGeometryNotCarried { .. }
        }
    ));
}

#[test]
fn non_geometric_update_requires_the_list_surface() {
    let snapshot = snapshot(date(2026, 1, 22), Vec::new());
    let update = AeronauticalUpdateV1 {
        update_id: "service-1".into(),
        display_text: "Satellite service unavailable".into(),
        subject: None,
        geometry: None,
    };

    let result = AirspaceViewV1::derive(&snapshot, &[update]);

    assert_eq!(
        result.updates[0].resolution,
        GeometryResolutionV1::NoSubjectGeometry
    );
    assert!(matches!(
        result.map_completeness,
        MapCompletenessV1::SupplementalOnly {
            updates_without_geometry: 1
        }
    ));
}

#[test]
fn stable_subject_identity_does_not_hide_cycle_scope() {
    let first = subject("cycle-a", SubjectFamilyV1::Navaid, " bos ");
    let second = subject("cycle-b", SubjectFamilyV1::Navaid, "BOS");

    assert_eq!(first.stable_subject_id(), second.stable_subject_id());
    assert_ne!(first.cycle, second.cycle);
}

#[test]
fn inconsistent_snapshot_identity_is_rejected() {
    let cycle = NavDataCycle::faa_nasr(date(2026, 1, 22)).expect("test cycle must be valid");
    let data = NavDataSnapshot::new(cycle, Vec::new());
    let error = IdentifiedNavdataSnapshotV1::try_new(
        NavdataIdentityV1 {
            cycle: "faa-nasr:2026-02-19".into(),
            snapshot_id: "snapshot".into(),
            snapshot_digest: "digest".into(),
        },
        data,
    )
    .expect_err("a mismatched cycle must fail");

    assert!(matches!(
        error,
        AirspaceViewError::SnapshotCycleMismatch { .. }
    ));
}

fn identified(data: NavDataSnapshot) -> IdentifiedNavdataSnapshotV1 {
    let identity = NavdataIdentityV1 {
        cycle: navdata_cycle_id(&data),
        snapshot_id: "snapshot".into(),
        snapshot_digest: "digest".into(),
    };
    IdentifiedNavdataSnapshotV1::try_new(identity, data)
        .expect("test identity must match the snapshot")
}

fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).expect("test date must be valid")
}
