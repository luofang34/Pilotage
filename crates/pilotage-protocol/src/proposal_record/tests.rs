use super::*;

fn proposal() -> wire::MissionProposal {
    wire::MissionProposal {
        proposal_id: "proposal-1".to_owned(),
        objective_id: "objective-1".to_owned(),
        summary: "fly the lines".to_owned(),
        requires: vec![FEATURE_SURVEY_LINE.to_owned()],
        ..wire::MissionProposal::default()
    }
}

/// `record` with `body` and the digest of `body`.
fn rebodied(record: &wire::MissionProposalRecord, body: Vec<u8>) -> wire::MissionProposalRecord {
    wire::MissionProposalRecord {
        digest: body_digest(&body).to_vec(),
        body,
        ..record.clone()
    }
}

#[test]
fn a_record_forwarded_unchanged_verifies() {
    let sealed = SealedProposal::seal(proposal());
    let forwarded =
        wire::MissionProposalRecord::decode(sealed.record().encode_to_vec().as_slice()).unwrap();
    let opened = SealedProposal::open(forwarded).unwrap();
    assert_eq!(opened.digest(), sealed.digest());
    assert_eq!(opened.proposal().summary, "fly the lines");
    assert_eq!(opened.proposal().digest, sealed.digest());
}

#[test]
fn verification_uses_the_received_bytes_not_a_new_encoding() {
    // The same fields, written in another order than prost writes them:
    // summary (10) before objective_id (2) before proposal_id (1).
    let reordered = wire::MissionProposal {
        summary: "fly the lines".to_owned(),
        ..wire::MissionProposal::default()
    }
    .encode_to_vec()
    .into_iter()
    .chain(
        wire::MissionProposal {
            objective_id: "objective-1".to_owned(),
            ..wire::MissionProposal::default()
        }
        .encode_to_vec(),
    )
    .chain(
        wire::MissionProposal {
            proposal_id: "proposal-1".to_owned(),
            ..wire::MissionProposal::default()
        }
        .encode_to_vec(),
    )
    .collect::<Vec<u8>>();
    let sealed = SealedProposal::seal(proposal());
    let received = rebodied(sealed.record(), reordered.clone());
    let opened = SealedProposal::open(received).unwrap();
    let mut decoded = opened.proposal().clone();
    decoded.digest.clear();
    assert_ne!(
        decoded.encode_to_vec(),
        reordered,
        "a new encoding writes other bytes"
    );
    assert_eq!(opened.digest(), body_digest(&reordered).as_slice());
}

#[test]
fn changed_bytes_are_a_new_identity_and_the_old_digest_no_longer_names_them() {
    let sealed = SealedProposal::seal(proposal());
    // An unknown field (tag 99, varint 1) appended to the body.
    let mut extended = sealed.record().body.clone();
    extended.extend_from_slice(&[0x98, 0x06, 0x01]);
    let stale = wire::MissionProposalRecord {
        body: extended.clone(),
        ..sealed.record().clone()
    };
    assert!(matches!(
        SealedProposal::open(stale),
        Err(RecordRefused::DigestMismatch)
    ));
    let renamed = SealedProposal::open(rebodied(sealed.record(), extended)).unwrap();
    assert_ne!(
        renamed.digest(),
        sealed.digest(),
        "the same meaning, another identity"
    );
    assert_eq!(renamed.proposal().summary, sealed.proposal().summary);
}

#[test]
fn an_unknown_format_or_feature_is_refused() {
    let sealed = SealedProposal::seal(proposal());
    let future = wire::MissionProposalRecord {
        format: 7,
        ..sealed.record().clone()
    };
    assert!(matches!(
        SealedProposal::open(future),
        Err(RecordRefused::UnknownFormat(7))
    ));
    let mut later = proposal();
    later
        .requires
        .push("pilotage.task.inspect-tower/3".to_owned());
    assert_eq!(
        unknown_features(&later, &[FEATURE_SURVEY_LINE]),
        ["pilotage.task.inspect-tower/3"]
    );
}
