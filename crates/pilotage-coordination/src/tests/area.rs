//! A team's member strips are parts of the team's area, not the area.

use pilotage_protocol::wire;
use wire::mission_intent::Action;

use super::*;

#[test]
fn a_members_strip_does_not_claim_the_teams_area_identity() {
    let mut team = team();
    let mut named = objective();
    if let Some(wire::mission_objective::Kind::Survey(survey)) = named.kind.as_mut() {
        survey.area_ref = Some(wire::MissionAreaRef {
            area_id: "graphic-7".to_owned(),
            revision: 3,
            source: "drawn on the map".to_owned(),
            derived: false,
        });
    }
    let reply = team.on_intent(
        &wire::MissionIntent {
            request_id: "p".to_owned(),
            action: Some(Action::Propose(named)),
        },
        &context("p"),
    );
    assert!(!reply.intents.is_empty());
    for intent in reply.intents {
        let Some(Action::Propose(given)) = intent.intent.action else {
            panic!("a member proposal");
        };
        let Some(wire::mission_objective::Kind::Survey(strip)) = given.kind else {
            panic!("a survey strip");
        };
        assert!(strip.area_ref.is_none(), "{}", intent.member);
    }
}
