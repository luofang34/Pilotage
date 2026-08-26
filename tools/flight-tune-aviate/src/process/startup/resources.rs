use std::path::Path;

use super::super::SupervisedProcessRequest;
use crate::AviateSupervisorError;
use crate::artifact;
use crate::supervisor::SupervisorBootstrap;

pub(super) struct LaunchArtifacts {
    pub(super) root: crate::document::AnchoredDirectory,
    pub(super) supervisor: crate::document::AnchoredExecutable,
    pub(super) target: crate::document::AnchoredExecutable,
}

pub(super) fn stage_launch_artifacts(
    request: &SupervisedProcessRequest,
) -> Result<LaunchArtifacts, AviateSupervisorError> {
    let root = artifact::create_artifact_root(&request.artifact_root)?;
    let supervisor = match artifact::stage_executable(
        &root,
        &request.supervisor_executable,
        request.supervisor_executable_digest,
        artifact::SUPERVISOR_ARTIFACT,
    ) {
        Ok(supervisor) => supervisor,
        Err(source) => {
            let cleanup = artifact::remove_artifact_root(&root);
            return combine_cleanup(source, cleanup);
        }
    };
    let target = match artifact::stage_executable(
        &root,
        &request.target_executable,
        request.target_executable_digest,
        artifact::TARGET_ARTIFACT,
    ) {
        Ok(target) => target,
        Err(source) => return cleanup_one_artifact(root, supervisor, source),
    };
    Ok(LaunchArtifacts {
        root,
        supervisor,
        target,
    })
}

pub(super) fn build_bootstrap(
    request: &SupervisedProcessRequest,
    artifacts: &LaunchArtifacts,
    runtime_root: crate::document::AnchoredDirectory,
    target_current_directory: crate::document::AnchoredDirectory,
    release_secret_digest: flight_tune::Digest,
) -> Result<SupervisorBootstrap, AviateSupervisorError> {
    Ok(SupervisorBootstrap {
        schema_version: crate::supervisor::BOOTSTRAP_SCHEMA_VERSION,
        run_intent_digest: request.run_intent_digest,
        release_secret_digest,
        supervisor_executable: artifacts.supervisor.clone(),
        target_executable: artifacts.target.clone(),
        artifact_root: artifacts.root.clone(),
        runtime_root,
        target_arguments: request.target_arguments.clone(),
        target_environment: request.target_environment.clone(),
        target_process_contract: request.target_process_contract.clone(),
        target_current_directory,
        startup_timeout_millis: super::duration_millis(request.startup_timeout)?,
        cleanup_timeout_millis: super::duration_millis(request.cleanup_timeout)?,
    })
}

pub(super) fn cleanup_launch_artifacts(
    artifacts: &LaunchArtifacts,
) -> Result<(), AviateSupervisorError> {
    artifact::remove_staged(&artifacts.root, &artifacts.target)?;
    artifact::remove_staged(&artifacts.root, &artifacts.supervisor)?;
    artifact::remove_artifact_root(&artifacts.root)
}

pub(super) fn cleanup_launch_artifacts_after_owner(
    artifacts: &LaunchArtifacts,
) -> Result<(), AviateSupervisorError> {
    if !path_exists(
        &artifacts.root.path,
        "inspect returned launch-artifact root",
    )? {
        return Ok(());
    }
    for executable in [&artifacts.target, &artifacts.supervisor] {
        if path_exists(
            &executable.path,
            "inspect returned staged launch executable",
        )? {
            artifact::remove_staged(&artifacts.root, executable)?;
        }
    }
    artifact::remove_artifact_root(&artifacts.root)
}

fn path_exists(path: &Path, operation: &'static str) -> Result<bool, AviateSupervisorError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(crate::inspection::io_error(operation, path, source)),
    }
}

fn cleanup_one_artifact<T>(
    root: crate::document::AnchoredDirectory,
    executable: crate::document::AnchoredExecutable,
    source: AviateSupervisorError,
) -> Result<T, AviateSupervisorError> {
    let cleanup = artifact::remove_staged(&root, &executable)
        .and_then(|()| artifact::remove_artifact_root(&root));
    combine_cleanup(source, cleanup)
}

fn combine_cleanup<T>(
    source: AviateSupervisorError,
    cleanup: Result<(), AviateSupervisorError>,
) -> Result<T, AviateSupervisorError> {
    match cleanup {
        Ok(()) => Err(source),
        Err(cleanup) => Err(AviateSupervisorError::StartupCleanup {
            source: Box::new(source),
            cleanup: Box::new(cleanup),
        }),
    }
}
