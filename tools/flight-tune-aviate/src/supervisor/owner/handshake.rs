use std::io::Write as _;
use std::time::{Duration, Instant};

use super::super::config::GateConfig;
use super::{
    OwnerEvent, PreparedOwner, TargetPublicationState, attest_target, cleanup, launch, process_io,
};
use crate::AviateSupervisorError;
use crate::document::{PROCESS_IDENTITY_NAME, TARGET_ATTESTATION_NAME};
use crate::protocol::{ArmedMessage, GateEvent, ReleaseMessage, TargetReleaseMessage, encode_line};
use crate::runtime_files::{PARENT_READY_SOCKET, socket_path};

pub(super) fn run_prepared(mut owner: PreparedOwner) -> Result<(), AviateSupervisorError> {
    let release = match wait_for_parent_release(&mut owner) {
        Ok(release) => release,
        Err(error) => return cleanup::finish_after_error(owner, error),
    };
    match authorize_target(&mut owner, &release) {
        Ok(()) => serve(owner),
        Err(error) => reject_and_finish(owner, error),
    }
}

fn reject_and_finish(
    owner: PreparedOwner,
    error: AviateSupervisorError,
) -> Result<(), AviateSupervisorError> {
    let notification = match &error {
        AviateSupervisorError::IdentityMismatch { detail } => send_target_rejection(&owner, detail),
        _ => Ok(()),
    };
    let error = match notification {
        Ok(()) => error,
        Err(notification) => AviateSupervisorError::ReleaseNotification {
            source: Box::new(error),
            notification: Box::new(notification),
        },
    };
    cleanup::finish_after_error(owner, error)
}

pub(super) fn arm_owner(
    owner: &mut PreparedOwner,
    release_secret_digest: flight_tune::Digest,
) -> Result<(), AviateSupervisorError> {
    let config = launch::gate_config(&owner.bootstrap, release_secret_digest)?;
    let gate_input = owner
        .gate_input
        .as_mut()
        .ok_or_else(|| AviateSupervisorError::protocol("the launch-gate input pipe is missing"))?;
    write_gate_config(gate_input, &config)?;
    let digest = owner
        .store
        .publish(PROCESS_IDENTITY_NAME, &owner.process_identity)?;
    owner.process_identity_digest = Some(digest);
    send_armed(owner, digest)
}

fn write_gate_config(
    gate_input: &mut std::process::ChildStdin,
    config: &GateConfig,
) -> Result<(), AviateSupervisorError> {
    gate_input
        .write_all(&encode_line(config)?)
        .and_then(|()| gate_input.flush())
        .map_err(|source| process_io("write launch-gate configuration", source))
}

fn wait_for_parent_release(
    owner: &mut PreparedOwner,
) -> Result<ReleaseMessage, AviateSupervisorError> {
    let timeout = Duration::from_millis(owner.bootstrap.startup_timeout_millis);
    match owner.events.recv_timeout(timeout) {
        Ok(OwnerEvent::ParentRelease(result)) => result,
        Ok(OwnerEvent::ParentClosed) => Err(AviateSupervisorError::protocol(
            "the parent closed before release authorization",
        )),
        Ok(OwnerEvent::Gate(Err(error))) => {
            owner.gate_failed = true;
            Err(error)
        }
        Ok(OwnerEvent::Gate(Ok(_))) => Err(AviateSupervisorError::protocol(
            "the launch gate ran before parent authorization",
        )),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(AviateSupervisorError::Timeout {
            operation: "wait for parent release authorization",
        }),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(
            AviateSupervisorError::protocol("the owner event channel closed"),
        ),
    }
}

fn authorize_target(
    owner: &mut PreparedOwner,
    release: &ReleaseMessage,
) -> Result<(), AviateSupervisorError> {
    if release.correlation_nonce != owner.process_identity.correlation_nonce
        || crate::lease_store::digest_bytes(release.release_secret.as_bytes())
            != owner.bootstrap.release_secret_digest
    {
        return Err(AviateSupervisorError::protocol(
            "the parent release capability is invalid",
        ));
    }
    let gate_input = owner
        .gate_input
        .as_mut()
        .ok_or_else(|| AviateSupervisorError::protocol("the launch-gate input pipe is missing"))?;
    gate_input
        .write_all(release.release_secret.as_bytes())
        .and_then(|()| gate_input.write_all(b"\n"))
        .and_then(|()| gate_input.flush())
        .map_err(|source| process_io("release exact launch gate", source))?;
    owner.target_release_sent = true;
    let started = wait_for_target_start(owner)?;
    owner.target_pid = Some(started.pid);
    if started.parent_closed {
        return Err(AviateSupervisorError::protocol(
            "the parent closed after target release authorization",
        ));
    }
    let attestation = attest_target(owner, started.pid)?;
    owner.target_publication = TargetPublicationState::Candidate(attestation.clone());
    let digest = owner.store.publish(TARGET_ATTESTATION_NAME, &attestation)?;
    owner.target_publication = TargetPublicationState::Published {
        attestation,
        digest,
    };
    send_target_ready(owner, digest)?;
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StartedTarget {
    pid: u32,
    parent_closed: bool,
}

fn wait_for_target_start(
    owner: &mut PreparedOwner,
) -> Result<StartedTarget, AviateSupervisorError> {
    let timeout = Duration::from_millis(owner.bootstrap.startup_timeout_millis);
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or(AviateSupervisorError::Timeout {
            operation: "wait for launch-gate target identity",
        })?;
    let events = &owner.events;
    let gate_failed = &mut owner.gate_failed;
    let gate_input = &mut owner.gate_input;
    receive_target_start(events, deadline, gate_failed, || {
        gate_input.take();
    })
}

fn receive_target_start(
    events: &std::sync::mpsc::Receiver<OwnerEvent>,
    deadline: Instant,
    gate_failed: &mut bool,
    mut request_containment: impl FnMut(),
) -> Result<StartedTarget, AviateSupervisorError> {
    let mut parent_closed = false;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let event = events
            .recv_timeout(remaining)
            .map_err(|error| match error {
                std::sync::mpsc::RecvTimeoutError::Timeout => AviateSupervisorError::Timeout {
                    operation: "wait for launch-gate target identity",
                },
                std::sync::mpsc::RecvTimeoutError::Disconnected => {
                    AviateSupervisorError::protocol("the owner event channel closed")
                }
            })?;
        match event {
            OwnerEvent::Gate(Ok(GateEvent::TargetStarted { pid })) => {
                return Ok(StartedTarget { pid, parent_closed });
            }
            OwnerEvent::Gate(Ok(GateEvent::TargetContained { .. })) => {
                return Err(AviateSupervisorError::protocol(
                    "the launch gate contained a target before it reported the target identity",
                ));
            }
            OwnerEvent::ParentClosed => {
                if !parent_closed {
                    request_containment();
                    parent_closed = true;
                }
            }
            OwnerEvent::ParentRelease(_) => {
                return Err(AviateSupervisorError::protocol(
                    "the parent sent duplicate release authorization",
                ));
            }
            OwnerEvent::Gate(Err(error)) => {
                *gate_failed = true;
                return Err(error);
            }
        }
    }
}

fn serve(mut owner: PreparedOwner) -> Result<(), AviateSupervisorError> {
    let event = match owner.events.recv() {
        Ok(event) => event,
        Err(_) => {
            return cleanup::finish_after_error(
                owner,
                AviateSupervisorError::protocol("the owner event channel closed"),
            );
        }
    };
    match event {
        OwnerEvent::ParentClosed => cleanup::finish_or_hold(owner),
        OwnerEvent::ParentRelease(_) => cleanup::finish_after_error(
            owner,
            AviateSupervisorError::protocol("the parent sent duplicate release authorization"),
        ),
        OwnerEvent::Gate(Ok(GateEvent::TargetStarted { .. })) => cleanup::finish_after_error(
            owner,
            AviateSupervisorError::protocol("the launch gate sent duplicate target identity"),
        ),
        OwnerEvent::Gate(Ok(GateEvent::TargetContained { pid })) => {
            if owner.target_pid == Some(pid) {
                owner.target_contained = true;
            }
            cleanup::finish_after_error(
                owner,
                AviateSupervisorError::protocol(
                    "the launch gate contained the target before parent closure",
                ),
            )
        }
        OwnerEvent::Gate(Err(error)) => {
            owner.gate_failed = true;
            cleanup::finish_after_error(owner, error)
        }
    }
}

fn send_armed(
    owner: &PreparedOwner,
    process_identity_digest: flight_tune::Digest,
) -> Result<(), AviateSupervisorError> {
    let message = ArmedMessage {
        correlation_nonce: owner.process_identity.correlation_nonce,
        spawn_intent_digest: owner.spawn_intent_digest,
        process_identity_digest,
    };
    send_parent_message(owner, &message)
}

fn send_target_ready(
    owner: &PreparedOwner,
    target_attestation_digest: flight_tune::Digest,
) -> Result<(), AviateSupervisorError> {
    let process_identity_digest = owner.process_identity_digest.ok_or_else(|| {
        AviateSupervisorError::invalid_document(
            "process identity",
            "the process identity was not durably published",
        )
    })?;
    let message = TargetReleaseMessage::Ready {
        correlation_nonce: owner.process_identity.correlation_nonce,
        process_identity_digest,
        target_attestation_digest,
    };
    send_parent_message(owner, &message)
}

fn send_target_rejection(owner: &PreparedOwner, detail: &str) -> Result<(), AviateSupervisorError> {
    send_parent_message(
        owner,
        &TargetReleaseMessage::RejectedIdentityMismatch {
            correlation_nonce: owner.process_identity.correlation_nonce,
            detail: detail.to_owned(),
        },
    )
}

#[cfg(test)]
#[path = "handshake/tests.rs"]
mod tests;

fn send_parent_message(
    owner: &PreparedOwner,
    message: &impl serde::Serialize,
) -> Result<(), AviateSupervisorError> {
    crate::protocol::connect_and_write(
        &socket_path(&owner.bootstrap.runtime_root.path, PARENT_READY_SOCKET),
        message,
        Duration::from_millis(owner.bootstrap.startup_timeout_millis),
    )
}
