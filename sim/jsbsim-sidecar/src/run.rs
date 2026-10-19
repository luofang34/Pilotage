//! Assembles the world, the model paths, and the session from the
//! command line, and serves PX4.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use pilotage_world::{ArchiveTerrain, MagneticEpoch, TerrainSource, UniformTerrain, World};

use crate::args::Args;
use crate::engine::ModelPaths;
use crate::sensors::DEFAULT_NOISE;
use crate::session::{SessionConfig, SessionError, serve_blocking};
use crate::simulation::Start;

/// Why the sidecar stopped.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    /// The terrain archive could not be opened.
    #[error("terrain archive {}: {source}", path.display())]
    Terrain {
        /// The archive.
        path: PathBuf,
        /// Why, boxed: SQLite's error would make every result large.
        #[source]
        source: Box<pilotage_terrain_query::TerrainQueryError>,
    },
    /// The session failed.
    #[error(transparent)]
    Session(#[from] SessionError),
}

/// Runs the sidecar until it fails.
///
/// # Errors
///
/// Returns [`RunError`] when the world, the model, or the link fails.
pub fn run_blocking(args: &Args) -> Result<(), RunError> {
    let aircraft_dir = args
        .aircraft_dir
        .clone()
        .unwrap_or_else(|| args.aircraft.bundled_dir());
    let epoch_unix_s = args.epoch_unix_s.unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| {
                i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
            })
    });
    let config = SessionConfig {
        port: args.port,
        aircraft: args.aircraft,
        paths: ModelPaths {
            root: args.jsbsim_root.clone(),
            aircraft_dir,
            model: args.aircraft.model().to_owned(),
        },
        start: Start {
            latitude_deg: args.latitude_deg,
            longitude_deg: args.longitude_deg,
            heading_deg: args.heading_deg,
        },
        seed: args.seed,
        noise: DEFAULT_NOISE,
        realtime_factor: args.realtime_factor,
        lockstep_timeout: args.lockstep_timeout,
    };
    let epoch = MagneticEpoch::from_unix_seconds(epoch_unix_s);
    match &args.terrain {
        Some(path) => {
            let terrain =
                ArchiveTerrain::open_blocking(path).map_err(|source| RunError::Terrain {
                    path: path.clone(),
                    source: Box::new(source),
                })?;
            serve(&config, World::new(args.site, terrain, epoch))
        }
        None => {
            let terrain = UniformTerrain {
                elevation_m: args.site.elevation_m,
            };
            serve(&config, World::new(args.site, terrain, epoch))
        }
    }
}

fn serve<T: TerrainSource>(config: &SessionConfig, mut world: World<T>) -> Result<(), RunError> {
    let sources = world.sources();
    tracing::info!(
        terrain = sources.terrain.0,
        magnetic_model = sources.magnetic_model.name,
        epoch = sources.epoch.decimal_year(),
        "world"
    );
    Ok(serve_blocking(config, &mut world)?)
}
