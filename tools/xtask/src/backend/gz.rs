//! What every Gazebo backend shares: the gz-transport partition that
//! keeps one session's simulator apart from another's.

use super::SessionContext;
use crate::conflicts::Claim;

/// The gz-transport partition of a session, or `None` for instance 0.
///
/// A partition is the only gz isolation that covers every topic: world
/// names scope the world's own topics, but the camera rig publishes
/// `/camera` and `/chase_camera` at the root. Instance 0 keeps the
/// default partition, so the single-session workflow (a bare
/// `gz topic -l`, the reset scripts) sees its world exactly as before.
/// The backend name is part of the partition so two backends on the same
/// instance index never share one.
pub(crate) fn partition(ctx: &SessionContext, backend: &str) -> Option<String> {
    (ctx.instance != 0).then(|| format!("pilotage-{backend}-{}", ctx.instance))
}

/// The `GZ_PARTITION` entry for every process of the session that speaks
/// gz-transport: the server, the readiness probe, the FC's bridge, and
/// the host's camera sidecar. A process left without it joins the
/// default partition and talks to another session's world.
pub(crate) fn partition_env(ctx: &SessionContext, backend: &str) -> Vec<(String, String)> {
    partition(ctx, backend)
        .map(|partition| ("GZ_PARTITION".to_owned(), partition))
        .into_iter()
        .collect()
}

/// The claim a session makes on gz-transport: the default partition must
/// be empty for a session that uses it. A session with a partition of its
/// own claims nothing here; its FC ports already make the instance unique.
pub(crate) fn partition_claims(ctx: &SessionContext, backend: &str) -> Vec<Claim> {
    if partition(ctx, backend).is_some() {
        Vec::new()
    } else {
        vec![Claim::DefaultGzPartition]
    }
}

#[cfg(test)]
mod tests;
