//! Offline navigation search and source-qualified route planning.

mod coordination;
mod error;
mod navigation;
mod route;
mod vehicle;

pub use coordination::{
    Assignment, CoordinatedPlan, CoordinationSummary, MissionParticipant, Swarm, TimingAssessment,
    TimingMember, TimingTarget, assess_coordination, export_revision,
};
pub use error::PlanningError;
pub use navigation::{
    NavigationDataset, NavigationIndex, NavigationPoint, NavigationSource, SearchResult,
    merge_navigation_matches,
};
pub use route::{RouteLeg, RoutePlan, RouteSummary, RouteWaypoint, evaluate_route};
pub use vehicle::{SpeedReference, VehicleProfile, VehicleProfileDocument};
