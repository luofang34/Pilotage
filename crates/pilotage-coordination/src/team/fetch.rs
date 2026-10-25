//! The full team proposal or drafted revision an operator fetches before it
//! approves one.

use super::TeamLane;

impl TeamLane {
    /// Answers a fetch with the full proposal `digest` names: the team
    /// plan, or a drafted revision; an empty digest names the plan.
    pub(super) fn on_fetch(&self, id: String, digest: &[u8]) -> super::TeamReply {
        let plan = self.proposal_record.as_ref();
        let revision = self.revision.as_ref().map(|r| &r.record);
        let found = [plan, revision]
            .into_iter()
            .flatten()
            .find(|r| digest.is_empty() || r.digest == digest)
            .cloned();
        let receipt = match found {
            Some(proposal) => self.receipt(id, true, String::new(), Some(proposal)),
            None => self.receipt(
                id,
                false,
                "the team holds no proposal with that digest".to_owned(),
                None,
            ),
        };
        super::TeamReply {
            receipt: Some(receipt),
            intents: Vec::new(),
        }
    }
}
