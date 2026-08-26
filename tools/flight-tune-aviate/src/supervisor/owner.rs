use std::process::{Child, ChildStdin};
use std::sync::mpsc;

use super::config::SupervisorBootstrap;
use crate::AviateSupervisorError;
use crate::document::{ProcessIdentity, ProcessIdentityDocument, TargetAttestation};
use crate::lease_store::LeaseStore;
use crate::protocol::{GateEvent, ReleaseMessage};

mod cleanup;
mod handshake;
mod launch;

pub(super) enum OwnerEvent {
    ParentClosed,
    ParentRelease(Result<ReleaseMessage, AviateSupervisorError>),
    Gate(Result<GateEvent, AviateSupervisorError>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum TargetPublicationState {
    NotAttempted,
    Candidate(TargetAttestation),
    Published {
        attestation: TargetAttestation,
        digest: flight_tune::Digest,
    },
}

pub(super) struct PreparedOwner {
    pub(super) bootstrap: SupervisorBootstrap,
    pub(super) store: LeaseStore,
    pub(super) spawn_intent_digest: flight_tune::Digest,
    pub(super) process_identity_digest: Option<flight_tune::Digest>,
    pub(super) target_publication: TargetPublicationState,
    pub(super) process_identity: ProcessIdentityDocument,
    pub(super) target_pid: Option<u32>,
    pub(super) target_release_sent: bool,
    pub(super) target_identity: Option<ProcessIdentity>,
    pub(super) target_contained: bool,
    pub(super) gate_failed: bool,
    pub(super) gate: Child,
    pub(super) gate_input: Option<ChildStdin>,
    pub(super) events: mpsc::Receiver<OwnerEvent>,
}

pub(super) fn run() -> Result<(), AviateSupervisorError> {
    let owner = launch::prepare()?;
    handshake::run_prepared(owner)
}

pub(super) fn process_io(operation: &'static str, source: std::io::Error) -> AviateSupervisorError {
    AviateSupervisorError::ProcessIo { operation, source }
}

pub(super) fn validate_gate_lifetime(owner: &PreparedOwner) -> Result<(), AviateSupervisorError> {
    let expected = &owner.process_identity.target_gate;
    if let Some(actual) = crate::inspection::inspect_lifetime(expected.pid)? {
        return crate::inspection::validate_absent_or_exact(expected, Some(actual), "launch gate")
            .map(|_| ());
    }
    let snapshot = crate::inspection::process_group_snapshot(expected.process_group)?;
    let expected_start = match expected.start {
        crate::document::ProcessStartIdentity::MacOs { start_abstime, .. } => Some(start_abstime),
        crate::document::ProcessStartIdentity::Linux { .. } => None,
    };
    if expected_start.is_some_and(|start| {
        snapshot
            .exited
            .iter()
            .any(|member| member.pid == expected.pid && member.start_abstime == start)
    }) {
        return Ok(());
    }
    Err(AviateSupervisorError::RecoveryBlocked {
        detail: "the held launch-gate lifetime cannot be proved".to_owned(),
    })
}

pub(super) fn attest_target(
    owner: &mut PreparedOwner,
    pid: u32,
) -> Result<TargetAttestation, AviateSupervisorError> {
    launch::attest_target(owner, pid)
}
