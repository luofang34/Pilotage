use navigate_contract::{GeodeticPosition, Ident, MAX_IDENT_BYTES, PlanRole, Waypoint};

use super::navigate_plan_name;

fn waypoints(north_deg: f64) -> Vec<Waypoint> {
    let ident = Ident::try_new("L01A").expect("a short name");
    vec![Waypoint::new(
        ident,
        GeodeticPosition::new(north_deg.to_radians(), 0.0, 100.0),
    )]
}

#[test]
fn two_plans_of_one_kind_have_two_navigate_names() {
    let first = navigate_plan_name("survey", "survey:a", PlanRole::Mission, &waypoints(47.0))
        .expect("a name");
    let second = navigate_plan_name("survey", "survey:b", PlanRole::Mission, &waypoints(47.0))
        .expect("a name");
    let moved = navigate_plan_name("survey", "survey:a", PlanRole::Mission, &waypoints(47.1))
        .expect("a name");
    assert_ne!(first, second);
    assert_ne!(first, moved);
    for name in [&first, &second, &moved] {
        assert!(name.starts_with("survey-"), "{name}");
        assert_eq!(name.len(), "survey-".len() + 16, "{name}");
        assert!(name.len() <= MAX_IDENT_BYTES);
    }
}
