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
    })
}

#[test]
fn another_revision_or_identity_of_an_area_changes_the_objective_with_the_same_vertices() {
    let before = survey(area("graphic-7", 3));
    for after in [
        survey(area("graphic-7", 4)),
        survey(area("graphic-8", 3)),
        survey(None),
    ] {
        let changes = classify(&before, &after);
        assert!(changes.has(ChangeClass::Objective), "{after:?}");
    }
    let same = classify(&before, &survey(area("graphic-7", 3)));
    assert!(!same.has(ChangeClass::Objective));
}

#[test]
fn what_an_area_ref_says_for_display_and_a_derived_name_are_no_content() {
    let mut drawn = area("graphic-7", 3);
    let before = survey(drawn.clone());
    if let Some(r) = drawn.as_mut() {
        r.source = "drawn".to_owned();
    }
    assert!(!classify(&before, &survey(drawn)).has(ChangeClass::Objective));
    let derived = survey(area("derived:nassau st", 0));
    for after in [survey(area("derived:nassau street", 0)), survey(None)] {
        assert!(
            !classify(&derived, &after).has(ChangeClass::Objective),
            "{after:?}"
        );
    }
}
