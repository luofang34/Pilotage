#![allow(clippy::expect_used, clippy::panic)]

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
