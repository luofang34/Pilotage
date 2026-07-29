//! Typed failures building a mission from a snapshot and a route.

use aerocontext_core::NavDataError;
use aerocontext_navdata::blob::BlobError;
use aerocontext_planning::route::RouteError;
use chrono::NaiveDate;
use navigate_contract::PlanValidationError;
use navigate_geodesy::GeodesyError;

/// Why a mission could not be built. Every variant carries the context
/// an operator needs to act on the refusal.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MissionBuildError {
    /// The navdata blob failed its container checks (magic, checksum,
    /// version, truncation) or did not decode.
    #[error("navdata blob rejected")]
    Blob(#[from] BlobError),
    /// A navdata cycle window could not be constructed.
    #[error("navdata cycle rejected")]
    Cycle(#[from] NavDataError),
    /// The route string did not expand against the snapshot; the cycle
    /// date tells the operator which data the expansion was judged on.
    #[error("route {route:?} did not expand (snapshot cycle effective {cycle})")]
    RouteExpansion {
        /// The route string as given.
        route: String,
        /// Effective date of the snapshot the expansion ran against.
        cycle: NaiveDate,
        /// The expansion failure.
        #[source]
        source: RouteError,
    },
    /// The route expanded to no geometry (e.g. procedure-only tokens).
    #[error("route {route:?} expanded to no points")]
    EmptyRoute {
        /// The route string as given.
        route: String,
    },
    /// The mission anchor cannot anchor a local tangent plane.
    #[error("mission anchor rejected by geodesy")]
    Geodesy(#[from] GeodesyError),
    /// The built flight plan failed structural validation.
    #[error("mission plan invalid")]
    PlanInvalid(#[from] PlanValidationError),
}
