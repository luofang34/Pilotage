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

/// `world` with its spherical-coordinates origin at `point`, and its
/// vehicle facing `point`'s heading.
///
/// Fails closed, naming the tag, unless each coordinate and the
/// [`SPAWN_FRAME`] occur exactly once: a world with several would move
/// one origin and keep another.
pub(crate) fn relocate_world(world: &str, point: &SpawnPoint) -> Result<String, &'static str> {
    let world = replace_single(world, "latitude_deg", point.latitude_deg)?;
    let world = replace_single(&world, "longitude_deg", point.longitude_deg)?;
    let world = replace_single(&world, "elevation", point.elevation_m)?;
    face_heading(&world, point.heading_deg)
}

/// The world frame every vehicle and its camera rig are posed relative
/// to, so one yaw turns them together.
const SPAWN_FRAME: &str = "<frame name=\"vehicle_spawn\">";

/// The yaw of an ENU world (counter-clockwise from east) that points a
/// vehicle's nose, its +x axis, at a true heading (clockwise from north).
pub(crate) fn enu_yaw_rad(heading_deg: f64) -> f64 {
    let yaw_deg = (90.0 - heading_deg).rem_euclid(360.0);
    let yaw_deg = if yaw_deg > 180.0 {
        yaw_deg - 360.0
    } else {
        yaw_deg
    };
    yaw_deg.to_radians()
}

/// `world` with the [`SPAWN_FRAME`] at the origin, turned to `heading_deg`.
fn face_heading(world: &str, heading_deg: f64) -> Result<String, &'static str> {
    const TAG: &str = "vehicle_spawn";
    if world.matches(SPAWN_FRAME).count() != 1 {
        return Err(TAG);
    }
    let frame = world.find(SPAWN_FRAME).ok_or(TAG)? + SPAWN_FRAME.len();
    let frame_end = frame + world[frame..].find("</frame>").ok_or(TAG)?;
    let body = &world[frame..frame_end];
    let (open, close) = ("<pose>", "</pose>");
    if body.matches(open).count() != 1 {
        return Err(TAG);
    }
    let start = frame + body.find(open).ok_or(TAG)? + open.len();
    let end = start + world[start..frame_end].find(close).ok_or(TAG)?;
    Ok(format!(
        "{}0 0 0 0 0 {:.7}{}",
        &world[..start],
        enu_yaw_rad(heading_deg),
        &world[end..]
    ))
}

/// The elevation, metres above mean sea level, of `world`'s single
/// spherical-coordinates origin.
pub(crate) fn world_elevation(world: &str) -> Result<f64, &'static str> {
    const TAG: &str = "elevation";
    let (open, close) = ("<elevation>", "</elevation>");
    if world.matches(open).count() != 1 {
        return Err(TAG);
    }
    let start = world.find(open).ok_or(TAG)? + open.len();
    let end = start + world[start..].find(close).ok_or(TAG)?;
    world[start..end].trim().parse().map_err(|_| TAG)
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
