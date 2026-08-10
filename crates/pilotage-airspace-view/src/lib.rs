//! Cycle-scoped geometry resolution for aeronautical updates.
//!
//! [`AirspaceViewV1`] derives a read-only result from one identified Navdata
//! snapshot and a set of updates. It keeps updates that have no map geometry.

mod error;
mod model;
mod resolve;

#[cfg(test)]
mod tests;

pub use error::AirspaceViewError;
pub use model::{
    AeronauticalUpdateV1, AirspaceViewItemV1, AirspaceViewResultV1, GeometryCoverageV1,
    GeometryResolutionV1, GeometryV1, IdentifiedNavdataSnapshotV1, MapCompletenessV1,
    NavdataIdentityV1, ResolutionFailureReasonV1, ResolvedGeometryV1, SubjectExtentV1,
    SubjectFamilyV1, SubjectIdentityV1, SubjectReferenceV1, UpdateGeometryV1, navdata_cycle_id,
};
pub use resolve::AirspaceViewV1;

/// Schema version for the first AirspaceView request and result contract.
pub const AIRSPACE_VIEW_SCHEMA_VERSION: u16 = 1;
