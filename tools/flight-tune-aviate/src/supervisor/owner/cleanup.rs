use std::time::{Duration, Instant};

use super::{
    OwnerEvent, PreparedOwner, TargetPublicationState, process_io, validate_gate_lifetime,
};
use crate::AviateSupervisorError;
use crate::artifact;
use crate::document::{
    SCHEMA_VERSION, TARGET_ATTESTATION_NAME, TERMINAL_RECEIPT_NAME, TargetAttestation,
    TerminalReceipt,
};
use crate::protocol::GateEvent;
use crate::runtime_files::{PARENT_READY_SOCKET, remove_exact_socket, socket_path};
use crate::supervisor::process_control::{self, ProcessGroupSignal};

pub(super) fn finish_or_hold(mut owner: PreparedOwner) -> Result<(), AviateSupervisorError> {
    let result = finish(&mut owner);
    match result {
        Ok(()) => Ok(()),
        Err(error) => hold_failed_cleanup(owner, error),
    }
}

pub(super) fn finish_after_error<T>(
    owner: PreparedOwner,
    error: AviateSupervisorError,
) -> Result<T, AviateSupervisorError> {
    match finish_or_hold(owner) {
        Ok(()) => Err(error),
        Err(cleanup) => Err(AviateSupervisorError::StartupCleanup {
            source: Box::new(error),
            cleanup: Box::new(cleanup),
        }),
    }
}

fn finish(owner: &mut PreparedOwner) -> Result<(), AviateSupervisorError> {
    request_gate_containment(owner)?;
    owner
        .gate
        .wait()
        .map_err(|source| process_io("reap exact launch-gate leader", source))?;
    wait_for_group_empty(owner)?;
    wait_for_exact_target_absent(owner)?;
    cleanup_runtime_and_artifacts(owner)?;
    let Some(process_identity_digest) = owner.process_identity_digest else {
        return Ok(());
    };
    let target_attestation_digest =
        resolve_target_publication(&owner.store, &mut owner.target_publication)?;
    let receipt = TerminalReceipt {
        schema_version: SCHEMA_VERSION,
        run_intent_digest: owner.bootstrap.run_intent_digest,
        spawn_intent_digest: owner.spawn_intent_digest,
        process_identity_digest,
        target_attestation_digest,
    };
    owner.store.publish(TERMINAL_RECEIPT_NAME, &receipt)?;
    Ok(())
}

fn resolve_target_publication(
    store: &crate::lease_store::LeaseStore,
    state: &mut TargetPublicationState,
) -> Result<Option<flight_tune::Digest>, AviateSupervisorError> {
    match state.clone() {
        TargetPublicationState::NotAttempted => {
            let actual: Option<(TargetAttestation, _)> =
                store.repair_optional(TARGET_ATTESTATION_NAME)?;
            if actual.is_some() {
                return Err(AviateSupervisorError::invalid_document(
                    "target attestation",
                    "a target attestation exists without a publication attempt",
                ));
            }
            Ok(None)
        }
        TargetPublicationState::Candidate(expected) => {
            let actual: Option<(TargetAttestation, _)> =
                store.repair_optional(TARGET_ATTESTATION_NAME)?;
            resolve_candidate(state, expected, actual)
        }
        TargetPublicationState::Published {
            attestation,
            digest,
        } => {
            let (actual, actual_digest): (TargetAttestation, _) =
                store.repair(TARGET_ATTESTATION_NAME)?;
            if actual != attestation || actual_digest != digest {
                return Err(AviateSupervisorError::invalid_document(
                    "target attestation",
                    "the published target attestation changed",
                ));
            }
            Ok(Some(digest))
        }
    }
}

fn resolve_candidate(
    state: &mut TargetPublicationState,
    expected: TargetAttestation,
    actual: Option<(TargetAttestation, flight_tune::Digest)>,
) -> Result<Option<flight_tune::Digest>, AviateSupervisorError> {
    let Some((attestation, digest)) = actual else {
        *state = TargetPublicationState::NotAttempted;
        return Ok(None);
    };
    if attestation != expected {
        return Err(AviateSupervisorError::invalid_document(
            "target attestation",
            "the repaired target attestation differs from the verified candidate",
        ));
    }
    *state = TargetPublicationState::Published {
        attestation,
        digest,
    };
    Ok(Some(digest))
}

fn request_gate_containment(owner: &mut PreparedOwner) -> Result<(), AviateSupervisorError> {
    owner.gate_input.take();
    if owner.target_pid.is_some() {
        return match wait_for_target_contained(owner) {
            Ok(()) => {
                wait_for_exact_target_absent(owner)?;
                stop_target_group(owner)
            }
            Err(initial) => contain_after_gate_failure(owner, initial),
        };
    }
    if owner.target_release_sent {
        return contain_after_gate_failure(
            owner,
            AviateSupervisorError::protocol(
                "the target release did not produce an exact target identity",
            ),
        );
    }
    if owner.gate_failed {
        return contain_after_gate_failure(
            owner,
            AviateSupervisorError::protocol(
                "the launch gate stopped before it reported the target identity",
            ),
        );
    }
    match wait_for_gate_quiescence(owner) {
        Ok(()) => Ok(()),
        Err(initial) => {
            if let Err(fallback) = signal_group(owner) {
                return Err(AviateSupervisorError::StartupCleanup {
                    source: Box::new(initial),
                    cleanup: Box::new(fallback),
                });
            }
            wait_for_gate_quiescence(owner)
        }
    }
}

fn contain_after_gate_failure(
    owner: &PreparedOwner,
    initial: AviateSupervisorError,
) -> Result<(), AviateSupervisorError> {
    tracing::warn!(%initial, "launch-gate confirmation failed; owner uses anchored group cleanup");
    match stop_target_group(owner).and_then(|()| wait_for_exact_target_absent(owner)) {
        Ok(()) => Ok(()),
        Err(cleanup) => Err(AviateSupervisorError::StartupCleanup {
            source: Box::new(initial),
            cleanup: Box::new(cleanup),
        }),
    }
}

fn stop_target_group(owner: &PreparedOwner) -> Result<(), AviateSupervisorError> {
    signal_group(owner)?;
    wait_for_gate_quiescence(owner)
}

fn wait_for_target_contained(owner: &mut PreparedOwner) -> Result<(), AviateSupervisorError> {
    if owner.target_contained {
        return Ok(());
    }
    if owner.gate_failed {
        return Err(AviateSupervisorError::protocol(
            "the launch gate stopped before containment confirmation",
        ));
    }
    let timeout = Duration::from_millis(owner.bootstrap.cleanup_timeout_millis);
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or(AviateSupervisorError::Timeout {
            operation: "wait for exact target containment",
        })?;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let event = owner
            .events
            .recv_timeout(remaining)
            .map_err(|error| match error {
                std::sync::mpsc::RecvTimeoutError::Timeout => AviateSupervisorError::Timeout {
                    operation: "wait for exact target containment",
                },
                std::sync::mpsc::RecvTimeoutError::Disconnected => {
                    AviateSupervisorError::protocol("the owner event channel closed")
                }
            })?;
        match event {
            OwnerEvent::Gate(Ok(GateEvent::TargetContained { pid })) => {
                if owner.target_pid != Some(pid) {
                    return Err(AviateSupervisorError::protocol(
                        "the launch gate contained another target PID",
                    ));
                }
                owner.target_contained = true;
                return Ok(());
            }
            OwnerEvent::ParentClosed => {}
            OwnerEvent::ParentRelease(_) => {
                return Err(AviateSupervisorError::protocol(
                    "the parent sent duplicate release authorization",
                ));
            }
            OwnerEvent::Gate(Ok(GateEvent::TargetStarted { .. })) => {
                return Err(AviateSupervisorError::protocol(
                    "the launch gate sent duplicate target identity",
                ));
            }
            OwnerEvent::Gate(Err(error)) => {
                owner.gate_failed = true;
                return Err(error);
            }
        }
    }
}

fn wait_for_exact_target_absent(owner: &PreparedOwner) -> Result<(), AviateSupervisorError> {
    let Some(pid) = owner.target_pid else {
        return Ok(());
    };
    let timeout = Duration::from_millis(owner.bootstrap.cleanup_timeout_millis);
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or(AviateSupervisorError::Timeout {
            operation: "wait for exact target removal",
        })?;
    loop {
        let actual = crate::inspection::inspect_lifetime(pid)?;
        match (&owner.target_identity, actual) {
            (_, None) => return Ok(()),
            (Some(expected), Some(actual)) => {
                crate::inspection::validate_absent_or_exact(expected, Some(actual), "target")?;
            }
            (None, Some(_)) => {
                return Err(AviateSupervisorError::RecoveryBlocked {
                    detail: "the target PID is live without an attested lifetime".to_owned(),
                });
            }
        }
        if Instant::now() >= deadline {
            return Err(AviateSupervisorError::Timeout {
                operation: "wait for exact target removal",
            });
        }
        std::thread::park_timeout(Duration::from_millis(1));
    }
}

fn signal_group(owner: &PreparedOwner) -> Result<(), AviateSupervisorError> {
    validate_gate_lifetime(owner)?;
    validate_target_group(owner)?;
    let group = owner.process_identity.target_gate.process_group;
    match process_control::signal_process_group(group)? {
        ProcessGroupSignal::Delivered => Ok(()),
        ProcessGroupSignal::GroupMissing
            if crate::inspection::process_group_snapshot(
                owner.process_identity.target_gate.process_group,
            )?
            .is_quiescent() =>
        {
            Ok(())
        }
        ProcessGroupSignal::GroupMissing => Err(AviateSupervisorError::RecoveryBlocked {
            detail: "the target process group disappeared before it became quiescent".to_owned(),
        }),
    }
}

fn validate_target_group(owner: &PreparedOwner) -> Result<(), AviateSupervisorError> {
    let expected = &owner.process_identity.target_gate;
    let snapshot = crate::inspection::process_group_snapshot(expected.process_group)?;
    let has_untrusted_member = !snapshot.unclassified_pids.is_empty()
        || snapshot.raw_pids.len() != snapshot.observed.len().wrapping_add(snapshot.exited.len())
        || snapshot.observed.iter().any(|member| {
            member.session_id != expected.session_id
                || member.real_user_id != expected.real_user_id
                || member.start.boot_identity() != expected.start.boot_identity()
        });
    if has_untrusted_member {
        return Err(AviateSupervisorError::RecoveryBlocked {
            detail: "the target group has a member outside the isolated launch session".to_owned(),
        });
    }
    Ok(())
}

fn wait_for_gate_quiescence(owner: &PreparedOwner) -> Result<(), AviateSupervisorError> {
    wait_for_members(owner, "wait for target group quiescence", |snapshot| {
        Ok(snapshot.is_quiescent())
    })
}

fn wait_for_group_empty(owner: &PreparedOwner) -> Result<(), AviateSupervisorError> {
    wait_for_members(owner, "wait for target group removal", |snapshot| {
        Ok(snapshot.is_empty()
            && crate::inspection::process_group_is_absent(
                owner.process_identity.target_gate.process_group,
            )?)
    })
}

fn wait_for_members(
    owner: &PreparedOwner,
    operation: &'static str,
    complete: impl Fn(&crate::inspection::ProcessGroupSnapshot) -> Result<bool, AviateSupervisorError>,
) -> Result<(), AviateSupervisorError> {
    let timeout = Duration::from_millis(owner.bootstrap.cleanup_timeout_millis);
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or(AviateSupervisorError::Timeout { operation })?;
    loop {
        let snapshot = crate::inspection::process_group_snapshot(
            owner.process_identity.target_gate.process_group,
        )?;
        if complete(&snapshot)? {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(AviateSupervisorError::Timeout { operation });
        }
        std::thread::park_timeout(Duration::from_millis(1));
    }
}

fn cleanup_runtime_and_artifacts(owner: &PreparedOwner) -> Result<(), AviateSupervisorError> {
    artifact::validate_directory(&owner.bootstrap.runtime_root, true)?;
    remove_exact_socket(&socket_path(
        &owner.bootstrap.runtime_root.path,
        PARENT_READY_SOCKET,
    ))?;
    crate::runtime_files::require_entries(&owner.bootstrap.runtime_root.path, &[])?;
    for executable in [
        &owner.bootstrap.target_executable,
        &owner.bootstrap.supervisor_executable,
    ] {
        artifact::remove_staged(&owner.bootstrap.artifact_root, executable)?;
    }
    artifact::remove_artifact_root(&owner.bootstrap.artifact_root)
}

fn hold_failed_cleanup(_owner: PreparedOwner, error: AviateSupervisorError) -> ! {
    tracing::error!(%error, "Aviate cleanup failed; the owner keeps the writer lease");
    loop {
        std::thread::park();
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic)]
mod tests;
