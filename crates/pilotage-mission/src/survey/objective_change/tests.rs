use pilotage_protocol::wire;

use super::classify;
use crate::change::ChangeClass;

fn survey(area_ref: Option<wire::MissionAreaRef>) -> wire::MissionObjective {
    wire::MissionObjective {
        kind: Some(wire::mission_objective::Kind::Survey(
            wire::SurveyObjective {
                area: vec![
                    wire::GeoPoint {
                        latitude_deg: 40.0,
                        longitude_deg: -74.0,
                    },
                    wire::GeoPoint {
                        latitude_deg: 40.0,
                        longitude_deg: -73.9,
                    },
                    wire::GeoPoint {
                        latitude_deg: 40.1,
                        longitude_deg: -73.9,
                    },
                ],
                area_ref,
                ..wire::SurveyObjective::default()
            },
        )),
        ..wire::MissionObjective::default()
    }
}

fn area(id: &str, revision: u64) -> Option<wire::MissionAreaRef> {
    Some(wire::MissionAreaRef {
        area_id: id.to_owned(),
        revision,
        source: "drawn on the map".to_owned(),
        derived: false,
    })
}

fn derived(name: &str) -> Option<wire::MissionAreaRef> {
    Some(wire::MissionAreaRef {
        derived: true,
        ..area(&format!("derived:{name}"), 0)?
    })
}

#[test]
fn another_named_area_changes_the_objective_with_the_same_vertices() {
    // Two areas drawn and never edited, each at revision 0.
    let before = survey(area("graphic-7", 0));
    for after in [
        survey(area("graphic-8", 0)),
        survey(None),
        survey(derived("x")),
    ] {
        assert!(
            classify(&before, &after).has(ChangeClass::Objective),
            "{after:?}"
        );
    }
}

#[test]
fn a_new_revision_with_the_same_vertices_and_what_a_ref_shows_are_no_content() {
    let before = survey(area("graphic-7", 3));
    let mut shown = area("graphic-7", 4);
    if let Some(r) = shown.as_mut() {
        r.source = "drawn".to_owned();
    }
    assert!(!classify(&before, &survey(shown)).has(ChangeClass::Objective));
    let derived_before = survey(derived("nassau st"));
    for after in [survey(derived("nassau street")), survey(None)] {
        assert!(
            !classify(&derived_before, &after).has(ChangeClass::Objective),
            "{after:?}"
        );
    }
}
