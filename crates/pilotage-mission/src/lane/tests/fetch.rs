//! A fetch returns the full proposal its digest names.

use pilotage_protocol::wire;

use super::*;

#[test]
fn a_fetch_returns_the_full_plan_the_digest_names() {
    use wire::mission_intent::Action;
    let (mut lane, _) = running_lane();
    let digest = lane.status().proposal_digest;
    let fetch = |digest: &[u8]| {
        intent(Action::Fetch(wire::MissionFetch {
            digest: digest.to_vec(),
        }))
    };
    let reply = lane.on_intent(&fetch(&digest), &context("f"));
    assert!(accepted(&reply));
    let plan = opened(&reply.receipt).expect("the plan");
    assert_eq!(plan.digest, digest);
    assert!(!plan.tasks.is_empty(), "the full plan, not a summary");
    assert!(!accepted(&lane.on_intent(&fetch(b"other"), &context("g"))));
}
