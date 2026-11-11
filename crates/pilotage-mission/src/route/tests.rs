use navigate_contract::{Ellipsoid, FlightPlan, GeodeticPosition, Ident, PlanRole, Waypoint};
use navigate_route::{ExecutionConfig, PlanExecution, SequenceEvent};

use super::MissionRoute;

fn waypoint(name: &str, north_deg: f64) -> Waypoint {
    let ident = Ident::try_new(name).expect("a short name");
    Waypoint::new(
        ident,
        GeodeticPosition::new(north_deg.to_radians(), 0.0, 100.0),
    )
}

fn waypoints() -> Vec<Waypoint> {
    vec![
        waypoint("A", 0.00),
        waypoint("B", 0.01),
        waypoint("C", 0.02),
        waypoint("D", 0.03),
    ]
}

/// Samples along the route: some far from every fix, some inside the
/// capture radius of the next one.
fn samples() -> Vec<GeodeticPosition> {
    (0..=40)
        .map(|step| GeodeticPosition::new((f64::from(step) * 0.00075).to_radians(), 0.0, 100.0))
        .collect()
}

#[test]
fn the_owned_route_sequences_like_a_borrowed_execution() {
    let storage = waypoints();
    let config = ExecutionConfig::default();
    let plan = FlightPlan::new(
        Ident::try_new("mission").expect("a short name"),
        PlanRole::Mission,
        Ellipsoid::WGS84,
        &storage,
    );
    let mut reference = PlanExecution::new(plan, config).expect("the plan activates");
    let mut route = MissionRoute::activate("mission", PlanRole::Mission, waypoints(), config)
        .expect("the plan activates");
    let mut advanced = 0;
    for position in samples() {
        let expected = reference.advance(&position, 20.0);
        let actual = route.advance(&position, 20.0);
        assert_eq!(actual, expected);
        assert_eq!(route.active_index(), reference.active_index());
        assert_eq!(
            route.active_leg().map(|leg| leg.index),
            reference.active_leg().map(|leg| leg.index)
        );
        assert_eq!(
            route.remaining_waypoints().len(),
            reference.remaining_waypoints().len()
        );
        if !matches!(actual, SequenceEvent::None) {
            advanced += 1;
        }
    }
    assert_eq!(advanced, storage.len(), "every fix sequenced once");
    assert!(route.active_leg().is_none(), "the route is complete");
    assert_eq!(route.advance(&samples()[0], 20.0), SequenceEvent::None);
}

#[test]
fn a_name_that_is_not_an_identifier_refuses_the_plan() {
    let long = "a-plan-name-that-is-longer-than-thirty-two-bytes";
    let refused = MissionRoute::activate(
        long,
        PlanRole::Mission,
        waypoints(),
        ExecutionConfig::default(),
    );
    assert!(matches!(
        refused,
        Err(crate::MissionBuildError::NavigationIdent { ref ident, .. }) if ident == long
    ));
}
