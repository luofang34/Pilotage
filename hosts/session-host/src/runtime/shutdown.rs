//! Graceful shutdown: signal every spawned task to stop, wait up to a
//! timeout, and abort whatever is left (ADR-0015's shutdown discipline).

use std::time::Duration;

use tokio::task::JoinSet;
use tracing::{error, info};

/// How long shutdown waits for spawned tasks to exit on their own before
/// aborting the stragglers.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

/// Awaits every task in `tasks` up to [`SHUTDOWN_TIMEOUT`], aborting and
/// logging by name whatever has not finished when the timeout elapses.
pub async fn join_with_timeout(mut tasks: JoinSet<()>) {
    let deadline = tokio::time::sleep(SHUTDOWN_TIMEOUT);
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            joined = tasks.join_next() => {
                match joined {
                    Some(Ok(())) => {}
                    Some(Err(error)) if error.is_cancelled() => {}
                    Some(Err(error)) => error!(%error, "task panicked during shutdown"),
                    None => {
                        info!("all tasks exited cleanly before shutdown timeout");
                        return;
                    }
                }
            }
            () = &mut deadline => {
                let stragglers = tasks.len();
                if stragglers > 0 {
                    error!(stragglers, "shutdown timeout elapsed, aborting remaining tasks");
                    tasks.shutdown().await;
                }
                return;
            }
        }
    }
}
