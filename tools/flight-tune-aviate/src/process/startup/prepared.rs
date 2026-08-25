use std::io::Write as _;
use std::process::ChildStdin;

use super::resources::LaunchArtifacts;
use super::{
    attestation, cleanup_socket_only, cleanup_started_failure, cleanup_unbootstrapped,
    cleanup_unstarted, random_release_secret, spawn_owner, wait_parent_message, write_bootstrap,
};
use crate::AviateSupervisorError;
use crate::artifact;
use crate::document::ProcessIdentity;
use crate::process::reaper::ReapableOwner;
use crate::process::{
    ManagedAviateProcess, RecoveryOutcome, SupervisedProcessRequest, SupervisionAttestation,
};
use crate::protocol::{
    ArmedMessage, ReleaseMessage, TargetReadyMessage, TargetReleaseMessage, encode_line,
};
use crate::runtime_files::{PARENT_READY_SOCKET, socket_path};
use crate::supervisor::SupervisorBootstrap;

pub(crate) struct PreparedLaunch {
    request: SupervisedProcessRequest,
    listener: std::os::unix::net::UnixListener,
    supervisor: Option<ReapableOwner>,
    parent_lifetime: Option<ChildStdin>,
    release_secret: String,
    correlation_nonce: flight_tune::Digest,
    attestation: SupervisionAttestation,
}

impl Drop for PreparedLaunch {
    fn drop(&mut self) {
        self.parent_lifetime.take();
    }
}

pub(crate) fn prepare_supported(
    request: SupervisedProcessRequest,
) -> Result<PreparedLaunch, AviateSupervisorError> {
    super::validate_request(&request)?;
    crate::runtime_files::validate_private_root(&request.runtime_root)?;
    crate::runtime_files::require_entries(&request.runtime_root, &[])?;
    let current_directory = artifact::inspect_directory(&request.target_current_directory, false)?;
    let runtime = artifact::inspect_directory(&request.runtime_root, true)?;
    let release_secret = random_release_secret()?;
    let release_digest = crate::lease_store::digest_bytes(release_secret.as_bytes());
    let listener_path = socket_path(&request.runtime_root, PARENT_READY_SOCKET);
    let listener = crate::runtime_files::bind_socket(&listener_path)?;
    let artifacts = match super::resources::stage_launch_artifacts(&request) {
        Ok(artifacts) => artifacts,
        Err(source) => return cleanup_socket_only(listener, &listener_path, source),
    };
    let bootstrap = match super::resources::build_bootstrap(
        &request,
        &artifacts,
        runtime,
        current_directory,
        release_digest,
    ) {
        Ok(bootstrap) => bootstrap,
        Err(source) => return cleanup_unstarted(listener, &listener_path, &artifacts, source),
    };
    prepare_staged(request, artifacts, listener, bootstrap, release_secret)
}

fn prepare_staged(
    request: SupervisedProcessRequest,
    artifacts: LaunchArtifacts,
    listener: std::os::unix::net::UnixListener,
    bootstrap: SupervisorBootstrap,
    release_secret: String,
) -> Result<PreparedLaunch, AviateSupervisorError> {
    let listener_path = socket_path(&request.runtime_root, PARENT_READY_SOCKET);
    let mut supervisor = match spawn_owner(&request, &artifacts) {
        Ok(supervisor) => supervisor,
        Err(source) => return cleanup_unstarted(listener, &listener_path, &artifacts, source),
    };
    let Some(mut parent_lifetime) = supervisor.child_mut()?.stdin.take() else {
        return cleanup_missing_pipe(listener, listener_path, artifacts, supervisor, &request);
    };
    if let Err(source) = write_bootstrap(&mut parent_lifetime, &bootstrap) {
        drop(parent_lifetime);
        return cleanup_unbootstrapped(
            listener,
            &listener_path,
            &artifacts,
            supervisor.child_mut()?,
            request.cleanup_timeout,
            source,
        );
    }
    finish_prepare(
        request,
        artifacts,
        listener,
        supervisor,
        parent_lifetime,
        release_secret,
    )
}

fn cleanup_missing_pipe(
    listener: std::os::unix::net::UnixListener,
    listener_path: std::path::PathBuf,
    artifacts: LaunchArtifacts,
    mut supervisor: ReapableOwner,
    request: &SupervisedProcessRequest,
) -> Result<PreparedLaunch, AviateSupervisorError> {
    cleanup_unbootstrapped(
        listener,
        &listener_path,
        &artifacts,
        supervisor.child_mut()?,
        request.cleanup_timeout,
        AviateSupervisorError::protocol("the owner parent-lifetime pipe is missing"),
    )
}

fn finish_prepare(
    request: SupervisedProcessRequest,
    artifacts: LaunchArtifacts,
    listener: std::os::unix::net::UnixListener,
    mut supervisor: ReapableOwner,
    parent_lifetime: ChildStdin,
    release_secret: String,
) -> Result<PreparedLaunch, AviateSupervisorError> {
    let armed: ArmedMessage =
        match wait_parent_message(&listener, supervisor.child_mut()?, request.startup_timeout) {
            Ok(message) => message,
            Err(source) => {
                drop(parent_lifetime);
                return cleanup_started_failure(
                    listener,
                    &socket_path(&request.runtime_root, PARENT_READY_SOCKET),
                    &artifacts,
                    supervisor.child_mut()?,
                    request.cleanup_timeout,
                    source,
                );
            }
        };
    let attestation =
        match attestation::validate_armed(&request, supervisor.id()?, &armed, &release_secret) {
            Ok(value) => value,
            Err(source) => {
                drop(parent_lifetime);
                return cleanup_started_failure(
                    listener,
                    &socket_path(&request.runtime_root, PARENT_READY_SOCKET),
                    &artifacts,
                    supervisor.child_mut()?,
                    request.cleanup_timeout,
                    source,
                );
            }
        };
    Ok(PreparedLaunch {
        request,
        listener,
        supervisor: Some(supervisor),
        parent_lifetime: Some(parent_lifetime),
        release_secret,
        correlation_nonce: armed.correlation_nonce,
        attestation,
    })
}

pub(crate) fn prepared_attestation(launch: &PreparedLaunch) -> &SupervisionAttestation {
    &launch.attestation
}

pub(crate) fn cancel_prepared(launch: &mut PreparedLaunch) {
    launch.parent_lifetime.take();
    tracing::warn!(
        supervisor_pid = launch.attestation.supervisor_identity.pid,
        "prepared Aviate launch dropped; target release remains closed"
    );
}

pub(crate) fn cancel_supported(
    mut launch: PreparedLaunch,
) -> Result<RecoveryOutcome, AviateSupervisorError> {
    launch.parent_lifetime.take();
    let supervisor = launch
        .supervisor
        .as_mut()
        .ok_or_else(|| AviateSupervisorError::protocol("the prepared process owner is missing"))?;
    super::wait_for_supervisor_terminal(
        supervisor.child_mut()?,
        &launch.attestation.supervisor_identity,
        launch.request.cleanup_timeout,
    )?;
    crate::process::recover_supervised_process_blocking(&launch.attestation.recovery_request)
}

pub(crate) fn release_supported(
    mut launch: PreparedLaunch,
) -> Result<ManagedAviateProcess, AviateSupervisorError> {
    let target_identity = match release_target(&mut launch) {
        Ok(identity) => identity,
        Err(source) => return fail_prepared_release(&mut launch, source),
    };
    let (supervisor, parent_lifetime) = take_managed_processes(&mut launch)?;
    Ok(ManagedAviateProcess {
        supervisor,
        parent_lifetime: Some(parent_lifetime),
        supervisor_identity: launch.attestation.supervisor_identity.clone(),
        target_identity,
        attestation: launch.attestation.clone(),
        recovery: launch.attestation.recovery_request.clone(),
        cleanup_timeout: launch.request.cleanup_timeout,
        terminated: false,
    })
}

fn release_target(launch: &mut PreparedLaunch) -> Result<ProcessIdentity, AviateSupervisorError> {
    let release = ReleaseMessage {
        correlation_nonce: launch.correlation_nonce,
        release_secret: launch.release_secret.clone(),
    };
    let parent_lifetime = launch.parent_lifetime.as_mut().ok_or_else(|| {
        AviateSupervisorError::protocol("the prepared parent-lifetime pipe is missing")
    })?;
    parent_lifetime
        .write_all(&encode_line(&release)?)
        .and_then(|()| parent_lifetime.flush())
        .map_err(|source| super::process_io("send parent release authorization", source))?;
    let supervisor = launch
        .supervisor
        .as_mut()
        .ok_or_else(|| AviateSupervisorError::protocol("the prepared process owner is missing"))?;
    let response: TargetReleaseMessage = wait_parent_message(
        &launch.listener,
        supervisor.child_mut()?,
        launch.request.startup_timeout,
    )?;
    let ready = parse_target_release(response, launch.correlation_nonce)?;
    attestation::validate_target_ready(
        &launch.request,
        &ready,
        &launch.attestation.supervisor_identity,
    )
}

fn parse_target_release(
    response: TargetReleaseMessage,
    expected_nonce: flight_tune::Digest,
) -> Result<TargetReadyMessage, AviateSupervisorError> {
    match response {
        TargetReleaseMessage::Ready {
            correlation_nonce,
            process_identity_digest,
            target_attestation_digest,
        } => Ok(TargetReadyMessage {
            correlation_nonce,
            process_identity_digest,
            target_attestation_digest,
        }),
        TargetReleaseMessage::RejectedIdentityMismatch {
            correlation_nonce,
            detail,
        } if correlation_nonce == expected_nonce && !detail.is_empty() => {
            Err(AviateSupervisorError::identity_mismatch(detail))
        }
        TargetReleaseMessage::RejectedIdentityMismatch { .. } => Err(
            AviateSupervisorError::protocol("the target rejection message is invalid"),
        ),
    }
}

fn fail_prepared_release<T>(
    launch: &mut PreparedLaunch,
    source: AviateSupervisorError,
) -> Result<T, AviateSupervisorError> {
    launch.parent_lifetime.take();
    let Some(supervisor) = launch.supervisor.as_mut() else {
        return Err(source);
    };
    let cleanup = super::wait_for_supervisor_terminal(
        supervisor.child_mut()?,
        &launch.attestation.supervisor_identity,
        launch.request.cleanup_timeout,
    )
    .and_then(|()| {
        match crate::process::recover_supervised_process_blocking(
            &launch.attestation.recovery_request,
        )? {
            RecoveryOutcome::Terminal { .. } => Ok(()),
            RecoveryOutcome::BootChange { .. } => Err(AviateSupervisorError::RecoveryBlocked {
                detail: "a boot-change receipt cannot confirm same-boot startup cleanup".to_owned(),
            }),
        }
    });
    match cleanup {
        Ok(()) => Err(source),
        Err(cleanup) => Err(AviateSupervisorError::StartupCleanup {
            source: Box::new(source),
            cleanup: Box::new(cleanup),
        }),
    }
}

fn take_managed_processes(
    launch: &mut PreparedLaunch,
) -> Result<(ReapableOwner, ChildStdin), AviateSupervisorError> {
    match (launch.supervisor.take(), launch.parent_lifetime.take()) {
        (Some(supervisor), Some(parent_lifetime)) => Ok((supervisor, parent_lifetime)),
        (supervisor, parent_lifetime) => {
            launch.supervisor = supervisor;
            launch.parent_lifetime = parent_lifetime;
            Err(AviateSupervisorError::protocol(
                "the prepared process ownership is incomplete",
            ))
        }
    }
}
