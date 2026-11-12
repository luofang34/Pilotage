//! Typed failures building a mission from a snapshot and a route.

use aerocontext_core::NavDataError;
use aerocontext_navdata::blob::BlobError;
use aerocontext_planning::route::RouteError;
use chrono::NaiveDate;
use navigate_contract::IdentError;
use navigate_geodesy::GeodesyError;
use navigate_route::PlanActivationError;
use pilotage_mission_core::CodecError;

use crate::survey::SurveyError;

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
    /// The route expanded to no geometry (e.g. a `DCT`-only route).
    #[error("route {route:?} expanded to no points")]
    EmptyRoute {
        /// The route string as given.
        route: String,
    },
    /// The route carries a SID/STAR procedure token. The expander
    /// recognizes such a token but contributes no geometry for it (no
    /// procedure engine), so flying the built mission would silently
    /// omit the procedure.
    #[error("route {route:?} carries unexecuted procedure(s): {procedures:?}")]
    UnexecutedProcedure {
        /// The route string as given.
        route: String,
        /// Each procedure token the expansion recognized but could not
        /// resolve to geometry, e.g. `"TRUKN2"` or `"BLUES2.IIU"`.
        procedures: Vec<String>,
    },
    /// The mission anchor cannot anchor a local tangent plane.
    #[error("mission anchor rejected by geodesy")]
    Geodesy(#[from] GeodesyError),
    /// The built flight plan could not be activated: a structural
    /// defect, an execution parameter that cannot be flown, or a
    /// waypoint constraint the sequencer refuses (an approach speed
    /// under the floor, a leg no longer than the capture radius).
    #[error("mission plan cannot be activated: {0}")]
    PlanActivation(#[from] PlanActivationError),
    /// The operational mission document did not validate or encode.
    #[error("mission document rejected")]
    MissionDocument(#[from] CodecError),
    /// The survey request did not become a plan.
    #[error("survey request rejected: {0}")]
    Survey(#[from] SurveyError),
    /// A waypoint or plan name is not a Navigate identifier: it is
    /// empty or longer than 32 bytes. The mission is refused; a name is
    /// never shortened.
    #[error("name {ident:?} is not a navigation identifier: {source}")]
    NavigationIdent {
        /// The name as the route, the survey, or the engine gave it.
        ident: String,
        /// Why the name is refused.
        #[source]
        source: IdentError,
    },
    /// A waypoint list holds no waypoint.
    #[error("the waypoint list is empty")]
    EmptyWaypointList,
    /// A waypoint of a list is not a position on the Earth: a value is
    /// not finite, or the latitude or longitude is out of range.
    #[error(
        "waypoint {index} is not on the Earth: latitude {latitude_deg}°, longitude {longitude_deg}°, altitude {altitude_m} m"
    )]
    WaypointPosition {
        /// The waypoint's index in fly order.
        index: usize,
        /// Its latitude, degrees.
        latitude_deg: f64,
        /// Its longitude, degrees.
        longitude_deg: f64,
        /// Its altitude, metres.
        altitude_m: f64,
    },
    /// A Navigate plan value has no canonical identity encoding.
    #[error("flight-plan identity cannot encode {field} value {value}")]
    PlanIdentity {
        /// The unsupported flight-plan field.
        field: &'static str,
        /// The unsupported value.
        value: String,
    },
    /// The mission is for a real vehicle and names simulation truth as its
    /// ownship source. Only a simulator has a truth oracle.
    #[error(
        "a real vehicle has no simulation truth: the simulation-truth ownship source needs the \
         simulator execution target"
    )]
    TruthOwnshipOnRealVehicle,
    /// The operator's declared start holds a statement no navigator can
    /// take.
    #[error("the declared start is refused")]
    DeclaredStart {
        /// The refused part.
        #[source]
        source: crate::DeclaredStartError,
    },
    /// A planned waypoint declares an implausible RNP value (`<= 0` or
    /// non-finite).
    #[error("waypoint {index} declares an invalid RNP {rnp_nm} NM (must be finite and > 0)")]
    WaypointRnp {
        /// The waypoint's index in fly order.
        index: usize,
        /// The rejected RNP value, nautical miles.
        rnp_nm: f64,
    },
    /// Evaluating WMM2025 magnetic variation for a magnetic course at a
    /// waypoint failed.
    #[error("waypoint {index} magnetic variation failed: {source}")]
    MagneticField {
        /// The waypoint's index in fly order.
        index: usize,
        /// Underlying geomagnetic model error.
        #[source]
        source: Box<pilotage_world::WorldError>,
    },
}
