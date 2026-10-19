//! `pilotage-maplibre-camera`: dials a session host's sim camera port and
//! renders the vehicle's cameras from the world model.

use std::path::PathBuf;
use std::process::ExitCode;

use pilotage_maplibre_camera::{
    args, calibration, error::CameraError, link, prewarm, producer, render::Camera,
    rig::CameraSpec, style,
};
use tracing::{error, info};

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,wgpu=warn,naga=warn")),
        )
        .init();
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    match run(&arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(failure) => {
            error!(%failure, usage = args::USAGE, "maplibre camera stopped");
            ExitCode::FAILURE
        }
    }
}

/// The tile cache: the given directory, or the platform cache directory's
/// `pilotage/maplibre-camera` (ADR-0044 cache class: derived data the
/// system may purge).
fn cache_dir(config: &args::Config) -> Result<String, CameraError> {
    let dir = match &config.cache_dir {
        Some(dir) => dir.clone(),
        None => directories::ProjectDirs::from("", "", "pilotage")
            .map(|dirs| dirs.cache_dir().join("maplibre-camera"))
            .ok_or(CameraError::NoCacheDir)?,
    };
    dir.to_str()
        .map(str::to_owned)
        .ok_or_else(|| CameraError::CachePath(PathBuf::from(&dir)))
}

fn run(arguments: &[String]) -> Result<(), CameraError> {
    let config = args::parse(arguments)?;
    let specs = [
        CameraSpec::fpv(config.fpv_size),
        CameraSpec::gimbal(config.gimbal_size),
    ];
    if let Some(dir) = &config.calibration_dir {
        let written = calibration::write_artifacts(dir, &specs)?;
        info!(count = written.len(), dir = %dir.display(), "calibrations written");
    }
    let style = style::load(&config)?;
    let cache = cache_dir(&config)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("maplibre-camera")
        .build()
        .map_err(CameraError::Runtime)?;
    if let Some((latitude, longitude)) = config.prewarm {
        runtime.block_on(prewarm::prewarm(&config, &cache, latitude, longitude));
    }
    let _entered = runtime.enter();
    let (ends, link_task) = link::spawn(config.host_port);
    info!(
        port = config.host_port,
        cache, "maplibre camera producer up"
    );
    let loader = maplibre::io::resource_loader::SharedLoader::new(
        pilotage_maplibre_camera::imagery::ImageryRouter::new(&config, &cache),
    );
    let handle = runtime.handle().clone();
    let fps = config.fps;
    // A map is bound to the thread that builds it, so the render thread
    // builds its own.
    let renderer = std::thread::Builder::new()
        .name("camera-render".to_owned())
        .spawn(move || {
            let _entered = handle.enter();
            let built = handle.block_on(async {
                let mut cameras = Vec::new();
                for spec in specs {
                    cameras.push(Camera::new(spec, style.clone(), loader.clone()).await?);
                }
                Ok::<_, CameraError>(cameras)
            });
            // The renderer's errors cannot leave the thread that owns the
            // GPU, so the cause is logged here.
            let cameras = built.map_err(|failure| error!(%failure, "camera setup failed"))?;
            producer::run_blocking(cameras, ends, fps, &handle);
            Ok(())
        })
        .map_err(CameraError::RenderThread)?;
    let joined = renderer.join();
    link_task.abort();
    joined
        .map_err(|_| CameraError::RenderThreadPanicked)?
        .map_err(|()| CameraError::CameraSetup)
}
