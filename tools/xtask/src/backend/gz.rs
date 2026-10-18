//! What every Gazebo backend shares: the gz-transport partition that
//! keeps one session's simulator apart from another's, and the world
//! origin that places its vehicle.

use super::{SessionContext, SpawnPoint, Stage};
use crate::conflicts::Claim;
use crate::error::XtaskError;

/// The name of the session's relocated world inside its run directory.
const RELOCATED_WORLD: &str = "world.sdf";

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
    partition_for(ctx.instance, backend)
}

/// [`partition`] for a session known only by its instance, as a reset
/// knows it.
pub(crate) fn partition_for(instance: u16, backend: &str) -> Option<String> {
    (instance != 0).then(|| format!("pilotage-{backend}-{instance}"))
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

/// Puts a gz session's vehicle at `point` by moving its world's origin.
///
/// Each session runs a world of its own, and the vehicle starts at that
/// world's origin, so moving the origin moves the vehicle on the map and
/// leaves it on its own field. The gz server loads a copy written into
/// the session's run directory; the source world stays as it is.
///
/// # Errors
///
/// Returns [`XtaskError::WorldOrigin`] when the world has no single
/// origin to move, and [`XtaskError::Io`] when it cannot be read or the
/// copy cannot be written.
pub(crate) fn spawn_in_world(
    ctx: &SessionContext,
    point: &SpawnPoint,
    stages: &mut [Stage],
) -> Result<(), XtaskError> {
    let Some(arg) = stages
        .iter_mut()
        .filter(|stage| stage.spec.name == "gazebo")
        .flat_map(|stage| stage.spec.args.iter_mut())
        .find(|arg| arg.ends_with(".sdf"))
    else {
        return Err(XtaskError::WorldOrigin {
            world: ctx.log_dir.clone(),
            tag: "gazebo stage world",
        });
    };
    let source = std::path::PathBuf::from(arg.as_str());
    let world = std::fs::read_to_string(&source).map_err(|source| XtaskError::Io {
        context: "reading the world to relocate",
        source,
    })?;
    let relocated = relocate_world(&world, point).map_err(|tag| XtaskError::WorldOrigin {
        world: source.clone(),
        tag,
    })?;
    let target = ctx.log_dir.join(RELOCATED_WORLD);
    std::fs::write(&target, relocated).map_err(|source| XtaskError::Io {
        context: "writing the relocated world",
        source,
    })?;
    *arg = target.display().to_string();
    Ok(())
}

/// `world` with its spherical-coordinates origin at `point`.
///
/// Fails closed, naming the tag, unless each coordinate occurs exactly
/// once: a world with several would move one origin and keep another.
pub(crate) fn relocate_world(world: &str, point: &SpawnPoint) -> Result<String, &'static str> {
    let world = replace_single(world, "latitude_deg", point.latitude_deg)?;
    replace_single(&world, "longitude_deg", point.longitude_deg)
}

fn replace_single(text: &str, tag: &'static str, value: f64) -> Result<String, &'static str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    if text.matches(&open).count() != 1 {
        return Err(tag);
    }
    let start = text.find(&open).ok_or(tag)? + open.len();
    let end = start + text[start..].find(&close).ok_or(tag)?;
    Ok(format!("{}{value:.9}{}", &text[..start], &text[end..]))
}

#[cfg(test)]
mod tests;
