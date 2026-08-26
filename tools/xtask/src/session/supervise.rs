//! Watching a running session, and putting back what a restart consumes.
//!
//! Split from the session's own file because a lifecycle and the loop that
//! watches it are different lengths of thing to read.

use super::{MAX_STAGE_RESTARTS, SUPERVISE_INTERVAL};
use crate::backend::{SessionContext, Stage};
use crate::error::XtaskError;
use crate::output::print_line;
use crate::process::ManagedChild;
use crate::readiness::await_ready;

/// Puts back what a restarting stage consumes, or reports that the operator
/// asked to stop while it was being put back.
///
/// `false` means the stop was requested and the caller should return cleanly;
/// the replacement is deliberately not spawned in that case.
///
/// The work runs off the runtime thread because it can WAIT — re-issuing a
/// handshake waits on a simulator the operator may have just closed, which is
/// the ordinary way a session ends. Run inline it would stop the runtime
/// polling, and the task watching for ctrl-c would stop with it.
async fn restore_for_restart(
    backend: &dyn crate::backend::SimBackend,
    ctx: &SessionContext,
    stage_name: &'static str,
    cancel: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<bool, XtaskError> {
    let Some(work) = backend.before_stage_restart(ctx, stage_name) else {
        return Ok(true);
    };
    let mut task = tokio::task::spawn_blocking(work);
    tokio::select! {
        done = &mut task => match done {
            Ok(result) => result.map(|()| true),
            Err(error) => Err(XtaskError::SimulatorCapability {
                capability: "re-establishing what the restarted stage consumes",
                detail: format!(
                    "the work did not finish: {error}. The replacement was not started."
                ),
            }),
        },
        _ = cancel.changed() => {
            print_line("");
            print_line("stopping the session...");
            Ok(false)
        }
    }
}

pub(super) async fn supervise(
    children: &mut [ManagedChild],
    stages: &[Stage],
    backend: &dyn crate::backend::SimBackend,
    ctx: &SessionContext,
    cancel: &mut tokio::sync::watch::Receiver<bool>,
) -> Result<(), XtaskError> {
    let mut fc_restarts: u32 = 0;
    loop {
        tokio::select! {
            _ = cancel.changed() => {
                print_line("");
                print_line("stopping the session...");
                return Ok(());
            }
            () = tokio::time::sleep(SUPERVISE_INTERVAL) => {
                for index in 0..children.len() {
                    let Err(death) = children[index].check_running() else {
                        continue;
                    };
                    let stage = &stages[index];
                    if stage.spec.name != "flight-controller" {
                        return Err(death);
                    }
                    fc_restarts = fc_restarts.wrapping_add(1);
                    if fc_restarts > MAX_STAGE_RESTARTS {
                        return Err(death);
                    }
                    print_line(&format!(
                        "flight-controller exited (reset or crash); restarting ({fc_restarts}/{MAX_STAGE_RESTARTS})..."
                    ));
                    // The replacement inherits the plan's argv, so anything
                    // the dead process CONSUMED has to be put back before it
                    // starts or the restart hands it a path to nothing.
                    if !restore_for_restart(backend, ctx, stage.spec.name, cancel).await? {
                        return Ok(());
                    }
                    // A replacement that spawns but never reports ready
                    // must not outlive the error return: it is not in
                    // `children`, so the caller's teardown would miss it.
                    let mut replacement = ManagedChild::spawn(&stage.spec)?;
                    let ready = tokio::select! {
                        ready = await_ready(&mut replacement, &stage.readiness) => ready,
                        _ = cancel.changed() => {
                            print_line("");
                            print_line("stopping the session...");
                            replacement.terminate_group();
                            return Ok(());
                        }
                    };
                    if let Err(error) = ready {
                        replacement.terminate_group();
                        return Err(error);
                    }
                    print_line("flight-controller ready");
                    children[index] = replacement;
                }
            }
        }
    }
}
