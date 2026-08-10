//! Stateless resolution of update subjects against one Navdata snapshot.

use aerocontext_core::{NavDataSnapshot, NavPoint, NavPointKind};

use crate::{
    AIRSPACE_VIEW_SCHEMA_VERSION, AeronauticalUpdateV1, AirspaceViewItemV1, AirspaceViewResultV1,
    GeometryCoverageV1, GeometryResolutionV1, GeometryV1, IdentifiedNavdataSnapshotV1,
    MapCompletenessV1, ResolutionFailureReasonV1, ResolvedGeometryV1, SubjectExtentV1,
    SubjectFamilyV1, SubjectIdentityV1, SubjectReferenceV1,
};

/// Stateless AirspaceView resolver.
#[derive(Debug, Clone, Copy, Default)]
pub struct AirspaceViewV1;

impl AirspaceViewV1 {
    /// Resolves all updates against exactly one immutable Navdata snapshot.
    #[must_use]
    pub fn derive(
        snapshot: &IdentifiedNavdataSnapshotV1,
        updates: &[AeronauticalUpdateV1],
    ) -> AirspaceViewResultV1 {
        let items: Vec<AirspaceViewItemV1> = updates
            .iter()
            .map(|update| derive_item(snapshot, update))
            .collect();
        let updates_without_geometry = items.iter().fold(0_u32, |count, item| {
            if item.geometry.is_none() {
                count.wrapping_add(1)
            } else {
                count
            }
        });
        AirspaceViewResultV1 {
            schema_version: AIRSPACE_VIEW_SCHEMA_VERSION,
            navdata_identity: snapshot.identity().clone(),
            updates: items,
            map_completeness: MapCompletenessV1::SupplementalOnly {
                updates_without_geometry,
            },
        }
    }
}

fn derive_item(
    snapshot: &IdentifiedNavdataSnapshotV1,
    update: &AeronauticalUpdateV1,
) -> AirspaceViewItemV1 {
    let subject_identity = update.subject.as_ref().map(SubjectIdentityV1::from);
    if let Some(direct) = &update.geometry {
        if let Some(reason) = direct_geometry_mismatch(update, &direct.coverage) {
            return item(
                update,
                subject_identity,
                None,
                GeometryResolutionV1::Unresolved { reason },
            );
        }
        return item(
            update,
            subject_identity,
            Some(ResolvedGeometryV1 {
                geometry: direct.geometry.clone(),
                coverage: direct.coverage.clone(),
            }),
            GeometryResolutionV1::Direct,
        );
    }
    let Some(subject) = &update.subject else {
        return item(
            update,
            subject_identity,
            None,
            GeometryResolutionV1::NoSubjectGeometry,
        );
    };
    match resolve_subject(snapshot, subject) {
        Ok(geometry) => item(
            update,
            subject_identity,
            Some(geometry),
            GeometryResolutionV1::ResolvedFromNavdata,
        ),
        Err(reason) => item(
            update,
            subject_identity,
            None,
            GeometryResolutionV1::Unresolved { reason },
        ),
    }
}

fn direct_geometry_mismatch(
    update: &AeronauticalUpdateV1,
    coverage: &GeometryCoverageV1,
) -> Option<ResolutionFailureReasonV1> {
    let subject = update.subject.as_ref()?;
    let matches = match (&subject.extent, coverage) {
        (SubjectExtentV1::Whole, GeometryCoverageV1::WholeSubject) => true,
        (subject_extent, GeometryCoverageV1::Partial { extent }) => subject_extent == extent,
        _ => false,
    };
    (!matches).then(|| ResolutionFailureReasonV1::DirectGeometryExtentMismatch {
        subject_extent: subject.extent.clone(),
        geometry_coverage: coverage.clone(),
    })
}

fn item(
    update: &AeronauticalUpdateV1,
    subject_identity: Option<SubjectIdentityV1>,
    geometry: Option<ResolvedGeometryV1>,
    resolution: GeometryResolutionV1,
) -> AirspaceViewItemV1 {
    AirspaceViewItemV1 {
        update_id: update.update_id.clone(),
        display_text: update.display_text.clone(),
        subject_identity,
        geometry,
        resolution,
    }
}

fn resolve_subject(
    snapshot: &IdentifiedNavdataSnapshotV1,
    subject: &SubjectReferenceV1,
) -> Result<ResolvedGeometryV1, ResolutionFailureReasonV1> {
    if subject.cycle != snapshot.identity().cycle {
        return Err(ResolutionFailureReasonV1::IdentifierFromAnotherCycle {
            subject_cycle: subject.cycle.clone(),
            snapshot_cycle: snapshot.identity().cycle.clone(),
        });
    }
    match subject.family {
        SubjectFamilyV1::Aerodrome | SubjectFamilyV1::Navaid | SubjectFamilyV1::Fix => {
            resolve_point(snapshot.snapshot(), subject)
        }
        SubjectFamilyV1::Runway => resolve_runway(snapshot.snapshot(), subject),
        SubjectFamilyV1::Airspace => resolve_airspace(snapshot.snapshot(), subject),
        SubjectFamilyV1::Procedure | SubjectFamilyV1::Service | SubjectFamilyV1::Other => {
            Err(ResolutionFailureReasonV1::SubjectFamilyNotCarried {
                family: subject.family,
            })
        }
    }
}

fn resolve_point(
    snapshot: &NavDataSnapshot,
    subject: &SubjectReferenceV1,
) -> Result<ResolvedGeometryV1, ResolutionFailureReasonV1> {
    let matches: Vec<&NavPoint> = snapshot
        .points
        .iter()
        .filter(|point| point_matches(point, subject))
        .collect();
    let point = one_match(subject, &matches)?;
    require_whole_extent(subject)?;
    Ok(ResolvedGeometryV1 {
        geometry: GeometryV1::Point {
            position: point.position,
        },
        coverage: GeometryCoverageV1::WholeSubject,
    })
}

fn point_matches(point: &NavPoint, subject: &SubjectReferenceV1) -> bool {
    point.ident.eq_ignore_ascii_case(&subject.identifier)
        && subject.region.as_deref().is_none_or(|region| {
            point
                .region
                .as_deref()
                .is_some_and(|point_region| point_region.eq_ignore_ascii_case(region))
        })
        && matches!(
            (&point.kind, subject.family),
            (NavPointKind::Airport, SubjectFamilyV1::Aerodrome)
                | (NavPointKind::Navaid, SubjectFamilyV1::Navaid)
                | (NavPointKind::Waypoint, SubjectFamilyV1::Fix)
        )
}

fn resolve_runway(
    snapshot: &NavDataSnapshot,
    subject: &SubjectReferenceV1,
) -> Result<ResolvedGeometryV1, ResolutionFailureReasonV1> {
    let parent = subject.parent_identifier.as_deref().unwrap_or("");
    let matches: Vec<_> = snapshot
        .runways
        .iter()
        .filter(|runway| {
            runway.airport_ident.eq_ignore_ascii_case(parent)
                && runway.designator.eq_ignore_ascii_case(&subject.identifier)
        })
        .collect();
    one_match(subject, &matches)?;
    match &subject.extent {
        SubjectExtentV1::Whole => Err(ResolutionFailureReasonV1::GeometryNotCarried {
            family: subject.family,
            identifier: subject.identifier.clone(),
        }),
        extent => Err(ResolutionFailureReasonV1::PartialGeometryNotCarried {
            family: subject.family,
            identifier: subject.identifier.clone(),
            extent: extent.clone(),
        }),
    }
}

fn resolve_airspace(
    snapshot: &NavDataSnapshot,
    subject: &SubjectReferenceV1,
) -> Result<ResolvedGeometryV1, ResolutionFailureReasonV1> {
    let matches: Vec<_> = snapshot
        .airspaces
        .iter()
        .filter(|airspace| {
            airspace
                .designator
                .eq_ignore_ascii_case(&subject.identifier)
        })
        .collect();
    let airspace = one_match(subject, &matches)?;
    require_whole_extent(subject)?;
    let area =
        airspace
            .bounds
            .clone()
            .ok_or_else(|| ResolutionFailureReasonV1::GeometryNotCarried {
                family: subject.family,
                identifier: subject.identifier.clone(),
            })?;
    Ok(ResolvedGeometryV1 {
        geometry: GeometryV1::Area { area },
        coverage: GeometryCoverageV1::WholeSubject,
    })
}

fn require_whole_extent(subject: &SubjectReferenceV1) -> Result<(), ResolutionFailureReasonV1> {
    match &subject.extent {
        SubjectExtentV1::Whole => Ok(()),
        extent => Err(ResolutionFailureReasonV1::PartialGeometryNotCarried {
            family: subject.family,
            identifier: subject.identifier.clone(),
            extent: extent.clone(),
        }),
    }
}

fn one_match<'a, T>(
    subject: &SubjectReferenceV1,
    matches: &[&'a T],
) -> Result<&'a T, ResolutionFailureReasonV1> {
    match matches {
        [] => Err(ResolutionFailureReasonV1::UnknownIdentifier {
            family: subject.family,
            identifier: subject.identifier.clone(),
        }),
        [value] => Ok(*value),
        _ => Err(ResolutionFailureReasonV1::AmbiguousMatch {
            family: subject.family,
            identifier: subject.identifier.clone(),
            matches: bounded_match_count(matches.len()),
        }),
    }
}

fn bounded_match_count(count: usize) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}
