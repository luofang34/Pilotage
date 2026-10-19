//! The producer's command line.

use std::path::PathBuf;

use thiserror::Error;

/// Public-domain USGS National Map orthoimagery: keyless, and free to show
/// in derived video. It covers the United States only; pass
/// `--imagery-url` for other places.
pub const DEFAULT_IMAGERY_URL: &str = "https://basemap.nationalmap.gov/arcgis/rest/services/USGSImageryOnly/MapServer/tile/{z}/{y}/{x}";
/// The deepest zoom the default imagery serves everywhere it covers.
pub const DEFAULT_IMAGERY_MAX_ZOOM: u8 = 16;
/// Terrarium-encoded elevation, the terrain Sokoly's own style draws.
pub const DEFAULT_DEM_URL: &str =
    "https://s3.amazonaws.com/elevation-tiles-prod/terrarium/{z}/{x}/{y}.png";
/// Frames per second each camera renders at most.
pub const DEFAULT_FPS: f64 = 15.0;

/// What the producer renders and where it sends it.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// The session host's sim camera port on loopback.
    pub host_port: u16,
    /// Raster imagery tile URL template, `{z}`, `{x}`, `{y}`.
    pub imagery_url: String,
    /// Deepest zoom the imagery source serves.
    pub imagery_max_zoom: u8,
    /// Terrarium elevation tile URL template.
    pub dem_url: String,
    /// A complete MapLibre style to render instead of imagery over terrain.
    pub style: Option<PathBuf>,
    /// Where map tiles are cached; the platform cache directory when unset.
    pub cache_dir: Option<PathBuf>,
    /// Where each camera's calibration artifacts are written, if anywhere.
    pub calibration_dir: Option<PathBuf>,
    /// Frames per second each camera renders at most.
    pub fps: f64,
    /// Forward camera image size, pixels.
    pub fpv_size: (u32, u32),
    /// Gimbal camera image size, pixels.
    pub gimbal_size: (u32, u32),
}

/// A command line the producer cannot run with.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ArgsError {
    /// An option this producer does not know.
    #[error("unknown option {0:?}")]
    Unknown(String),
    /// An option given without its value.
    #[error("{0} needs a value")]
    MissingValue(&'static str),
    /// A value that does not parse for its option.
    #[error("{option} value {value:?} is invalid: {reason}")]
    Invalid {
        /// The option.
        option: &'static str,
        /// The refused value.
        value: String,
        /// What a valid value is.
        reason: &'static str,
    },
    /// `--host-port` was not given.
    #[error("--host-port is required: the session host's PILOTAGE_SIM_CAMERA_PORT")]
    NoHostPort,
}

/// The usage text.
pub const USAGE: &str = "pilotage-maplibre-camera --host-port PORT [--imagery-url URL] \
[--imagery-max-zoom Z] [--dem-url URL] [--style FILE] [--cache-dir DIR] \
[--calibration-dir DIR] [--fps N] [--fpv-size WxH] [--gimbal-size WxH]";

/// Parses the arguments after the program name.
///
/// # Errors
///
/// [`ArgsError`] for an unknown option, a missing or invalid value, or no
/// `--host-port`.
pub fn parse(args: &[String]) -> Result<Config, ArgsError> {
    let mut host_port = None;
    let mut config = Config {
        host_port: 0,
        imagery_url: DEFAULT_IMAGERY_URL.to_owned(),
        imagery_max_zoom: DEFAULT_IMAGERY_MAX_ZOOM,
        dem_url: DEFAULT_DEM_URL.to_owned(),
        style: None,
        cache_dir: None,
        calibration_dir: None,
        fps: DEFAULT_FPS,
        fpv_size: (640, 480),
        gimbal_size: (640, 480),
    };
    let mut iter = args.iter();
    while let Some(option) = iter.next() {
        let mut value = |name: &'static str| iter.next().ok_or(ArgsError::MissingValue(name));
        match option.as_str() {
            "--host-port" => host_port = Some(port(value("--host-port")?)?),
            "--imagery-url" => config.imagery_url = template(value("--imagery-url")?)?,
            "--imagery-max-zoom" => config.imagery_max_zoom = zoom(value("--imagery-max-zoom")?)?,
            "--dem-url" => config.dem_url = template(value("--dem-url")?)?,
            "--style" => config.style = Some(PathBuf::from(value("--style")?)),
            "--cache-dir" => config.cache_dir = Some(PathBuf::from(value("--cache-dir")?)),
            "--calibration-dir" => {
                config.calibration_dir = Some(PathBuf::from(value("--calibration-dir")?));
            }
            "--fps" => config.fps = fps(value("--fps")?)?,
            "--fpv-size" => config.fpv_size = size("--fpv-size", value("--fpv-size")?)?,
            "--gimbal-size" => config.gimbal_size = size("--gimbal-size", value("--gimbal-size")?)?,
            other => return Err(ArgsError::Unknown(other.to_owned())),
        }
    }
    config.host_port = host_port.ok_or(ArgsError::NoHostPort)?;
    Ok(config)
}

fn invalid(option: &'static str, value: &str, reason: &'static str) -> ArgsError {
    ArgsError::Invalid {
        option,
        value: value.to_owned(),
        reason,
    }
}

fn port(value: &str) -> Result<u16, ArgsError> {
    match value.parse::<u16>() {
        Ok(port) if port != 0 => Ok(port),
        _ => Err(invalid("--host-port", value, "a port from 1 to 65535")),
    }
}

fn template(value: &str) -> Result<String, ArgsError> {
    let complete = ["{z}", "{x}", "{y}"].iter().all(|key| value.contains(key));
    if complete {
        Ok(value.to_owned())
    } else {
        Err(invalid(
            "a tile URL",
            value,
            "a template with {z}, {x} and {y}",
        ))
    }
}

fn zoom(value: &str) -> Result<u8, ArgsError> {
    match value.parse::<u8>() {
        Ok(zoom) if zoom <= 22 => Ok(zoom),
        _ => Err(invalid("--imagery-max-zoom", value, "a zoom from 0 to 22")),
    }
}

fn fps(value: &str) -> Result<f64, ArgsError> {
    match value.parse::<f64>() {
        Ok(fps) if fps.is_finite() && fps > 0.0 && fps <= 120.0 => Ok(fps),
        _ => Err(invalid("--fps", value, "a rate above 0 and at most 120")),
    }
}

fn size(option: &'static str, value: &str) -> Result<(u32, u32), ArgsError> {
    let parsed = value.split_once('x').and_then(|(width, height)| {
        Some((width.parse::<u32>().ok()?, height.parse::<u32>().ok()?))
    });
    match parsed {
        Some((width, height)) if (16..=4096).contains(&width) && (16..=4096).contains(&height) => {
            Ok((width, height))
        }
        _ => Err(invalid(option, value, "WIDTHxHEIGHT, each from 16 to 4096")),
    }
}

#[cfg(test)]
mod tests;
