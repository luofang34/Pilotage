use pilotage_protocol::{Generation, PrincipalId, wire};
use pilotage_session::DomainEnvelope;
use tokio::sync::{mpsc, watch};

use super::super::{MISSION_SCOPE, MissionTask};
use crate::runtime::engine_actor::ToEngine;
use crate::runtime::{HOST_VEHICLE, automation::AutomationStatus};

fn task() -> (
    MissionTask,
    watch::Receiver<AutomationStatus>,
    mpsc::Sender<ToEngine>,
) {
    let (engine_tx, _engine_rx) = mpsc::channel(4);
    let (status_tx, status_rx) = watch::channel(AutomationStatus::default());
    let mut task = MissionTask::without_plan(
        engine_tx.downgrade(),
        tokio::time::Instant::now(),
        status_tx,
    );
    task.principal = Some(PrincipalId::new(9));
    task.generation = Some(Generation::new(3));
    (task, status_rx, engine_tx)
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

#[tokio::test]
async fn a_request_for_the_held_scope_is_answered_with_an_offer() {
    let (mut task, status, _engine_tx) = task();
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
    assert_eq!(status.borrow().offers, 1);
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

#[tokio::test]
async fn a_transfer_away_holds_and_a_transfer_back_resumes() {
    let (mut task, status, _engine_tx) = task();
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
    assert!(status.borrow().held);
    assert_eq!(status.borrow().lease_generation, None);

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
    assert!(!status.borrow().held);
    assert_eq!(status.borrow().resumes, 1);
    assert_eq!(status.borrow().lease_generation, Some(5));
}

#[tokio::test]
async fn revocation_override_and_a_foreign_grant_hold_rather_than_fence() {
    for case in 0..3 {
        let (mut task, status, _engine_tx) = task();
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
        assert!(status.borrow().held && !status.borrow().fenced);
    }
}

#[tokio::test]
async fn an_offer_to_another_principal_is_ignored_and_a_fenced_task_stays_fenced() {
    let (mut task, status, _engine_tx) = task();
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
    assert_eq!(status.borrow().lease_generation, None);
    let requested = wire::ScopeTransferRequested {
        from_principal: principal(21),
        vehicle: vehicle(),
        scope: scope(),
    };
    assert!(task.on_transfer_requested(&requested).is_none());
}

/// A mission flying under its lease after the launch: it asked for the
/// lease once the aircraft was airborne, and holds it.
fn flying() -> (
    MissionTask,
    watch::Receiver<AutomationStatus>,
    mpsc::Sender<ToEngine>,
) {
    let (mut task, status, engine_tx) = task();
    task.lease_wanted = true;
    (task, status, engine_tx)
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

#[tokio::test]
async fn control_lost_after_the_launch_comes_back_only_by_a_hand_back() {
    for case in ["native takeover", "own lease revoked", "emergency override"] {
        let (mut task, _status, _engine_tx) = flying();
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

#[tokio::test]
async fn an_operator_who_took_over_and_disconnected_does_not_return_control() {
    let (mut task, _status, _engine_tx) = flying();
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
