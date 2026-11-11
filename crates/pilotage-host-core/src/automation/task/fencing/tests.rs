use pilotage_protocol::{Generation, PrincipalId, wire};
use pilotage_session::DomainEnvelope;

use super::super::{MISSION_SCOPE, MissionTask};
use crate::HOST_VEHICLE;

fn task() -> MissionTask {
    let mut task = MissionTask::without_plan();
    task.principal = Some(PrincipalId::new(9));
    task.generation = Some(Generation::new(3));
    task
}

fn vehicle() -> Option<wire::VehicleId> {
    Some(wire::VehicleId {
        value: HOST_VEHICLE.as_u64(),
    })
}

fn scope() -> Option<wire::ScopeId> {
    Some(wire::ScopeId {
        value: MISSION_SCOPE.to_owned(),
    })
}

fn principal(value: u64) -> Option<wire::PrincipalId> {
    Some(wire::PrincipalId { value })
}

#[test]
fn a_request_for_the_held_scope_is_answered_with_an_offer() {
    let mut task = task();
    let requested = wire::ScopeTransferRequested {
        from_principal: principal(21),
        vehicle: vehicle(),
        scope: scope(),
    };
    let reply = task
        .on_transfer_requested(&requested)
        .expect("the holder offers");
    assert!(matches!(
        reply,
        DomainEnvelope::TransferOffer(offer) if offer.to == PrincipalId::new(21)
            && offer.vehicle == HOST_VEHICLE
            && offer.scope.as_str() == MISSION_SCOPE
    ));
    assert_eq!(task.status.offers, 1);
    assert!(!task.held, "the offer alone changes nothing");
    assert_eq!(task.generation, Some(Generation::new(3)));

    // The principal's own request, and a request while nothing is held,
    // get no offer.
    let own = wire::ScopeTransferRequested {
        from_principal: principal(9),
        ..requested.clone()
    };
    assert!(task.on_transfer_requested(&own).is_none());
    task.generation = None;
    assert!(task.on_transfer_requested(&requested).is_none());
}

#[test]
fn a_transfer_away_holds_and_a_transfer_back_resumes() {
    let mut task = task();
    let away = wire::ScopeTransferCommitted {
        from_principal: principal(9),
        to_principal: principal(21),
        vehicle: vehicle(),
        scope: scope(),
        generation: Some(wire::Generation { value: 4 }),
        ..Default::default()
    };
    task.on_transfer_event(&away);
    assert!(task.held && !task.fenced);
    assert_eq!(task.generation, None);
    assert!(task.status.held);
    assert_eq!(task.status.lease_generation, None);

    let offered = wire::ScopeTransferOffered {
        from_principal: principal(21),
        to_principal: principal(9),
        vehicle: vehicle(),
        scope: scope(),
        ..Default::default()
    };
    let reply = task
        .on_transfer_offered(&offered)
        .expect("an offer back is accepted");
    assert!(matches!(reply, DomainEnvelope::TransferAccept(_)));
    assert!(task.held, "acceptance is not the resume");

    let back = wire::ScopeTransferCommitted {
        from_principal: principal(21),
        to_principal: principal(9),
        vehicle: vehicle(),
        scope: scope(),
        generation: Some(wire::Generation { value: 5 }),
        ..Default::default()
    };
    task.on_transfer_event(&back);
    assert!(!task.held);
    assert_eq!(task.generation, Some(Generation::new(5)));
    assert_eq!(task.neutral_frames, 1, "the first resumed frame is neutral");
    assert!(!task.status.held);
    assert_eq!(task.status.resumes, 1);
    assert_eq!(task.status.lease_generation, Some(5));
}

#[test]
fn revocation_override_and_a_foreign_grant_hold_rather_than_fence() {
    for case in 0..3 {
        let mut task = task();
        match case {
            0 => task.on_revoked_event(&wire::ScopeLeaseRevoked {
                principal: principal(9),
                vehicle: vehicle(),
                scope: scope(),
                ..Default::default()
            }),
            1 => task.on_override_event(&wire::EmergencyOverrideApplied {
                principal: principal(30),
                vehicle: vehicle(),
                scope: scope(),
                ..Default::default()
            }),
            _ => task.on_granted_event(&wire::ScopeLeaseGranted {
                principal: principal(30),
                vehicle: vehicle(),
                scope: scope(),
                generation: Some(wire::Generation { value: 7 }),
                ..Default::default()
            }),
        }
        assert!(task.held, "case {case} holds");
        assert!(!task.fenced, "case {case} does not fence");
        assert!(task.status.held && !task.status.fenced);
    }
}

#[test]
fn an_offer_to_another_principal_is_ignored_and_a_fenced_task_stays_fenced() {
    let mut task = task();
    let elsewhere = wire::ScopeTransferOffered {
        from_principal: principal(21),
        to_principal: principal(22),
        vehicle: vehicle(),
        scope: scope(),
        ..Default::default()
    };
    assert!(task.on_transfer_offered(&elsewhere).is_none());

    task.fence("test fence");
    let granted = wire::ScopeLeaseGranted {
        vehicle: vehicle(),
        scope: scope(),
        principal: principal(9),
        generation: Some(wire::Generation { value: 5 }),
        ..Default::default()
    };
    task.on_granted_event(&granted);
    assert!(task.fenced, "fencing is permanent for the task");
    assert_eq!(task.generation, None);
    assert_eq!(task.status.lease_generation, None);
    let requested = wire::ScopeTransferRequested {
        from_principal: principal(21),
        vehicle: vehicle(),
        scope: scope(),
    };
    assert!(task.on_transfer_requested(&requested).is_none());
}

/// A mission flying under its lease after the launch: it asked for the
/// lease once the aircraft was airborne, and holds it.
fn flying() -> MissionTask {
    let mut task = task();
    task.lease_wanted = true;
    task
}

/// Whether the task would ask for the lease on any later tick.
fn asks_again(task: &MissionTask) -> bool {
    task.lease_due || task.lease_retry_due(u64::MAX)
}

fn released(by: u64) -> wire::ScopeLeaseRevoked {
    wire::ScopeLeaseRevoked {
        principal: principal(by),
        vehicle: vehicle(),
        scope: scope(),
        ..Default::default()
    }
}

#[test]
fn control_lost_after_the_launch_comes_back_only_by_a_hand_back() {
    for case in ["native takeover", "own lease revoked", "emergency override"] {
        let mut task = flying();
        match case {
            "native takeover" => task.on_granted_event(&wire::ScopeLeaseGranted {
                principal: principal(30),
                vehicle: vehicle(),
                scope: scope(),
                generation: Some(wire::Generation { value: 7 }),
                ..Default::default()
            }),
            "own lease revoked" => task.on_revoked_event(&released(9)),
            _ => task.on_override_event(&wire::EmergencyOverrideApplied {
                principal: principal(30),
                vehicle: vehicle(),
                scope: scope(),
                ..Default::default()
            }),
        }
        assert!(task.held && task.generation.is_none(), "{case}: held");
        assert!(!asks_again(&task), "{case}: no retry is scheduled");
        // The holder goes away, as an operator whose connection closed:
        // the lease is free, and the mission still does not take it.
        task.on_revoked_event(&released(30));
        assert!(!asks_again(&task), "{case}: a release is not a hand-back");
        assert!(task.held, "{case}");
        // An explicit hand-back returns control.
        let offered = wire::ScopeTransferOffered {
            from_principal: principal(30),
            to_principal: principal(9),
            vehicle: vehicle(),
            scope: scope(),
            ..Default::default()
        };
        assert!(matches!(
            task.on_transfer_offered(&offered),
            Some(DomainEnvelope::TransferAccept(_))
        ));
        task.on_transfer_event(&wire::ScopeTransferCommitted {
            from_principal: principal(30),
            to_principal: principal(9),
            vehicle: vehicle(),
            scope: scope(),
            generation: Some(wire::Generation { value: 8 }),
            ..Default::default()
        });
        assert!(!task.held, "{case}: the hand-back resumes");
        assert_eq!(task.generation, Some(Generation::new(8)), "{case}");
    }
}

#[test]
fn an_operator_who_took_over_and_disconnected_does_not_return_control() {
    let mut task = flying();
    // The operator takes the scope by transfer, then the connection closes.
    task.on_transfer_event(&wire::ScopeTransferCommitted {
        from_principal: principal(9),
        to_principal: principal(21),
        vehicle: vehicle(),
        scope: scope(),
        generation: Some(wire::Generation { value: 4 }),
        ..Default::default()
    });
    assert!(task.held && !asks_again(&task));
    task.on_revoked_event(&released(21));
    assert!(task.held, "the mission stays held");
    assert!(!asks_again(&task), "a lost operator hands nothing back");
}

#[test]
fn control_is_freed_only_when_the_taker_lets_go_without_a_hand_back() {
    let mut task = flying();
    task.on_granted_event(&wire::ScopeLeaseGranted {
        principal: principal(30),
        vehicle: vehicle(),
        scope: scope(),
        generation: Some(wire::Generation { value: 7 }),
        ..Default::default()
    });
    assert!(task.status.awaiting_hand_back);
    assert!(!task.status.control_freed, "the taker holds control");
    // Another principal's lease ends: that frees nothing.
    task.on_revoked_event(&released(31));
    assert!(!task.status.control_freed);
    // The taker's lease ends without a hand-back.
    task.on_revoked_event(&released(30));
    assert!(task.status.control_freed, "no one holds the lease");
    // Someone takes the lease again: control is not free any more.
    task.on_granted_event(&wire::ScopeLeaseGranted {
        principal: principal(32),
        vehicle: vehicle(),
        scope: scope(),
        generation: Some(wire::Generation { value: 9 }),
        ..Default::default()
    });
    assert!(!task.status.control_freed);
    // A hand-back returns control and clears the loss.
    task.on_transfer_event(&wire::ScopeTransferCommitted {
        from_principal: principal(32),
        to_principal: principal(9),
        vehicle: vehicle(),
        scope: scope(),
        generation: Some(wire::Generation { value: 10 }),
        ..Default::default()
    });
    assert!(!task.status.awaiting_hand_back && !task.status.control_freed);
}

#[test]
fn an_emergency_override_that_ends_frees_control_and_a_revocation_does_not() {
    let mut task = flying();
    task.on_override_event(&wire::EmergencyOverrideApplied {
        principal: principal(30),
        vehicle: vehicle(),
        scope: scope(),
        ..Default::default()
    });
    assert_eq!(task.status.lease_holder, Some(30), "the override holds");
    task.on_revoked_event(&released(30));
    assert!(task.status.control_freed);

    // The run's own lease is revoked, with no taker: a stop, never freed.
    let mut task = flying();
    task.on_revoked_event(&released(9));
    assert!(task.status.awaiting_hand_back);
    task.on_revoked_event(&released(30));
    assert!(!task.status.control_freed, "a revocation is a stop");
}

#[test]
fn a_takers_lost_connection_frees_control_as_a_release_does() {
    let mut task = flying();
    task.on_granted_event(&wire::ScopeLeaseGranted {
        principal: principal(30),
        vehicle: vehicle(),
        scope: scope(),
        generation: Some(wire::Generation { value: 7 }),
        ..Default::default()
    });
    let lost = |of: u64| wire::WarningRaised {
        vehicle: vehicle(),
        scope: scope(),
        kind: wire::WarningKind::HolderLinkLost as i32,
        principal: principal(of),
        ..Default::default()
    };
    // Another principal's link frees nothing; other warnings change nothing.
    task.on_warning_event(&lost(31));
    task.on_warning_event(&wire::WarningRaised {
        kind: wire::WarningKind::CommandRejected as i32,
        principal: principal(30),
        ..lost(30)
    });
    assert!(!task.status.control_freed);
    assert_eq!(task.status.lease_holder, Some(30));
    // The taker's connection closed: the engine released its scope.
    task.on_warning_event(&lost(30));
    assert!(task.status.control_freed && task.status.lease_holder.is_none());
    assert!(task.held, "freed is not resumed");
}
