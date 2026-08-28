//! One idempotent revoke removes every direct authority.

use flight_tune::ControlChannel;

use super::super::DirectTransportError;
use super::sender::RecordingSender;
use super::{authorize, capability, frozen, step_request, vehicle_receipt};

#[test]
fn one_revoke_removes_every_direct_authority() {
    let (mut transport, mut sender) = frozen();
    let prepared = transport
        .prepare_step(&step_request(ControlChannel::Roll, 1.0))
        .expect("prepared step");

    let receipt = transport.revoke();

    assert!(receipt.removed_authority());
    assert!(receipt.released_baseline());
    assert!(transport.is_revoked());
    assert!(
        transport.baseline().is_none(),
        "the frozen baseline is gone"
    );
    assert!(matches!(
        transport.prepare_step(&step_request(ControlChannel::Roll, 1.0)),
        Err(DirectTransportError::Revoked)
    ));
    assert!(matches!(
        transport.prepare_release(&step_request(ControlChannel::Roll, 1.0)),
        Err(DirectTransportError::Revoked)
    ));
    assert!(matches!(
        transport.freeze_baseline_blocking(&mut sender, &super::baseline_request()),
        Err(DirectTransportError::Revoked)
    ));
    assert!(matches!(
        transport.require_binding(&capability(), &vehicle_receipt()),
        Err(DirectTransportError::Revoked)
    ));
    assert!(matches!(
        transport.enact_blocking(&mut sender, &prepared),
        Err(DirectTransportError::Revoked)
    ));
    assert!(
        sender.transmitted().is_empty(),
        "a revoked transport commands nothing"
    );
}

#[test]
fn a_second_revoke_removes_nothing_and_returns_the_same_receipt() {
    let (mut transport, _sender) = frozen();

    let first = transport.revoke();
    let second = transport.revoke();
    let third = transport.revoke();

    assert!(first.removed_authority());
    assert!(!second.removed_authority());
    assert!(!second.released_baseline());
    assert_eq!(second, third, "revoke is idempotent");
    assert_eq!(
        first.transport_identity_digest(),
        second.transport_identity_digest(),
        "the receipt keeps naming the same transport"
    );
}

#[test]
fn a_revoke_before_any_baseline_still_removes_the_authority() {
    let sender = RecordingSender::new();
    let mut transport = authorize(&sender);

    let receipt = transport.revoke();

    assert!(receipt.removed_authority());
    assert!(
        !receipt.released_baseline(),
        "there was no frozen baseline to release"
    );
    assert!(transport.is_revoked());
}

#[test]
fn a_command_prepared_before_a_revoke_cannot_be_enacted_after_it() {
    let (mut transport, mut sender) = frozen();
    let prepared = transport
        .prepare_step(&step_request(ControlChannel::Vertical, 0.5))
        .expect("prepared step");

    transport.revoke();
    let result = transport.enact_blocking(&mut sender, &prepared);

    assert!(matches!(result, Err(DirectTransportError::Revoked)));
    assert!(sender.transmitted().is_empty());
}
