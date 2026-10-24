//! A revision inside the envelope is admitted only when the envelope permits
//! every class of change it makes: the plan, the assignment and the
//! objective, found by comparing the old and new content.

use pilotage_protocol::wire;
use wire::mission_intent::Action;

use super::*;

/// A running lane whose approval permits the plan changes `revise` and the
/// objective changes `objective`.
fn lane_permitting(revise: bool, objective: bool) -> Lane {
    let mut lane = Lane::new(config());
    let reply = lane.on_intent(&intent(Action::Propose(super::objective())), &context("p"));
    let digest = reply.receipt.proposal.expect("a proposal").digest;
    let mut approved = approval(&digest, 120.0);
    if let Some(envelope) = approved.envelope.as_mut() {
        envelope.permit_revise = revise;
        envelope.permit_change_objective = objective;
    }
    assert!(accepted(
        &lane.on_intent(&intent(Action::Approve(approved)), &context("a"))
    ));
    lane.on_intent(
        &intent(Action::Start(wire::MissionStart {
            proposal_digest: digest,
        })),
        &context("run-1"),
    );
    lane.on_progress(&toward(&lane, 0));
    lane
}

/// The objective with `edit` applied to its survey.
fn revised(edit: impl FnOnce(&mut wire::SurveyObjective)) -> wire::MissionObjective {
    let mut objective = objective();
    if let Some(wire::mission_objective::Kind::Survey(survey)) = objective.kind.as_mut() {
        edit(survey);
    }
    objective
}

/// Asks for `objective` inside the envelope; the successor's stage after it.
fn revise_within(
    lane: &mut Lane,
    objective: wire::MissionObjective,
) -> wire::MissionSuccessorState {
    let status = lane.status();
    let reply = lane.on_intent(
        &intent(Action::Revise(wire::MissionRevise {
            base_digest: status.proposal_digest.clone(),
            run_id: status.run_id.clone(),
            within_envelope: true,
            drafted_by: None,
            change: Some(wire::mission_revise::Change::Objective(objective)),
        })),
        &context("rev"),
    );
    assert!(accepted(&reply), "{}", reply.receipt.reason);
    let successor = lane.status().successor.expect("a draft");
    wire::MissionSuccessorState::try_from(successor.state).unwrap()
}

fn larger_area(survey: &mut wire::SurveyObjective) {
    survey.area.iter_mut().for_each(|p| {
        if p.latitude_deg > 40.347 {
            p.latitude_deg += 0.0005;
        }
    });
}

#[test]
fn a_new_area_needs_the_objective_permission() {
    let mut lane = lane_permitting(true, false);
    let stage = revise_within(&mut lane, revised(larger_area));
    assert_eq!(stage, wire::MissionSuccessorState::Drafted);
    let detail = lane.status().detail;
    assert!(
        detail.contains("permit_change_objective for the area"),
        "{detail}"
    );
}

#[test]
fn an_objective_and_plan_change_needs_both_permissions() {
    let both = |survey: &mut wire::SurveyObjective| {
        larger_area(survey);
        survey.cruise_mps = 8.0;
    };
    let mut lane = lane_permitting(false, true);
    assert_eq!(
        revise_within(&mut lane, revised(both)),
        wire::MissionSuccessorState::Drafted
    );
    assert!(
        lane.status().detail.contains("permit_revise"),
        "{}",
        lane.status().detail
    );
    let mut lane = lane_permitting(true, true);
    assert_ne!(
        revise_within(&mut lane, revised(both)),
        wire::MissionSuccessorState::Drafted
    );
}

#[test]
fn the_same_objective_identity_with_other_content_is_an_objective_change() {
    let mut lane = lane_permitting(true, false);
    let same_id = revised(|survey| survey.min_usable_frames_per_line = 4);
    assert_eq!(same_id.objective_id, objective().objective_id);
    assert_eq!(
        revise_within(&mut lane, same_id),
        wire::MissionSuccessorState::Drafted
    );
    assert!(
        lane.status()
            .detail
            .contains("the evidence each line needs")
    );
}

#[test]
fn a_plan_change_needs_only_the_revise_permission() {
    let mut lane = lane_permitting(true, false);
    let stage = revise_within(&mut lane, revised(|survey| survey.cruise_mps = 8.0));
    assert_ne!(
        stage,
        wire::MissionSuccessorState::Drafted,
        "{}",
        lane.status().detail
    );
}

#[test]
fn a_held_height_is_part_of_the_objective() {
    let lower = |hold: bool| {
        revised(move |survey| {
            survey.height_m = 50.0;
            survey.hold_height = hold;
        })
    };
    let mut lane = lane_permitting(true, false);
    assert_ne!(
        revise_within(&mut lane, lower(false)),
        wire::MissionSuccessorState::Drafted,
        "without a hold the height is the plan's: {}",
        lane.status().detail
    );
    let mut lane = lane_permitting(true, false);
    assert_eq!(
        revise_within(&mut lane, lower(true)),
        wire::MissionSuccessorState::Drafted
    );
}
