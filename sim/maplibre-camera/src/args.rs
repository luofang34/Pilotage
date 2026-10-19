//! The producer's command line.

use std::path::PathBuf;

use thiserror::Error;

/// The cached USGS National Map imagery basemap: public domain, keyless,
/// free to show in derived video, and served fast from a CDN, but only to
/// zoom 16 (about 1.8 m a pixel). It covers the United States.
pub const DEFAULT_BASE_IMAGERY_URL: &str = "https://basemap.nationalmap.gov/arcgis/rest/services/USGSImageryOnly/MapServer/tile/{z}/{y}/{x}";
/// The deepest zoom of the default base imagery.
pub const DEFAULT_BASE_IMAGERY_MAX_ZOOM: u8 = 16;
/// USDA NAIP orthoimagery from the USGS National Map: public domain and
/// keyless, with the 0.3 m pixels a zoomed payload camera resolves. It is a
/// dynamic image server, slow and sometimes failing on a cold tile, so it
/// is drawn only past the base imagery's zooms, over it: a tile it has not
/// delivered shows the base beneath. It covers the contiguous United
/// States only.
pub const DEFAULT_IMAGERY_URL: &str = concat!(
    "https://imagery.nationalmap.gov/arcgis/rest/services/USGSNAIPImagery/ImageServer/",
    "exportImage?bbox={bbox-epsg-3857}&bboxSR=3857&imageSR=3857&size=256,256",
    "&format=jpg&f=image",
);
/// The first zoom the detail imagery is drawn at.
pub const DEFAULT_IMAGERY_MIN_ZOOM: u8 = 17;
/// The zoom whose pixels match the detail imagery's 0.3 m ground pixels.
pub const DEFAULT_IMAGERY_MAX_ZOOM: u8 = 19;
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
    /// Base imagery tile URL template, drawn at every zoom it serves.
    pub base_imagery_url: String,
    /// Deepest zoom the base imagery serves.
    pub base_imagery_max_zoom: u8,
    /// Detail imagery tile URL template, drawn over the base from
    /// `imagery_min_zoom`.
    pub imagery_url: String,
    /// First zoom the detail imagery is drawn at.
    pub imagery_min_zoom: u8,
    /// Deepest zoom the detail imagery serves.
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
    /// A site whose tiles are cached before the cameras start, degrees.
    pub prewarm: Option<(f64, f64)>,
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
[--imagery-min-zoom Z] [--imagery-max-zoom Z] [--base-imagery-url URL] [--base-imagery-max-zoom Z] [--dem-url URL] [--style FILE] [--cache-dir DIR] \
[--calibration-dir DIR] [--fps N] [--fpv-size WxH] [--gimbal-size WxH] [--prewarm LAT,LON]";

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
        base_imagery_url: DEFAULT_BASE_IMAGERY_URL.to_owned(),
        base_imagery_max_zoom: DEFAULT_BASE_IMAGERY_MAX_ZOOM,
        imagery_url: DEFAULT_IMAGERY_URL.to_owned(),
        imagery_min_zoom: DEFAULT_IMAGERY_MIN_ZOOM,
        imagery_max_zoom: DEFAULT_IMAGERY_MAX_ZOOM,
        dem_url: DEFAULT_DEM_URL.to_owned(),
        style: None,
        cache_dir: None,
        calibration_dir: None,
        fps: DEFAULT_FPS,
        fpv_size: (640, 480),
        gimbal_size: (640, 480),
        prewarm: None,
    };
    let mut iter = args.iter();
    while let Some(option) = iter.next() {
        let mut value = |name: &'static str| iter.next().ok_or(ArgsError::MissingValue(name));
        match option.as_str() {
            "--host-port" => host_port = Some(port(value("--host-port")?)?),
            "--imagery-url" => config.imagery_url = template(value("--imagery-url")?)?,
            "--imagery-min-zoom" => {
                config.imagery_min_zoom = zoom("--imagery-min-zoom", value("--imagery-min-zoom")?)?;
            }
            "--imagery-max-zoom" => {
                config.imagery_max_zoom = zoom("--imagery-max-zoom", value("--imagery-max-zoom")?)?;
            }
            "--base-imagery-url" => {
                config.base_imagery_url = template(value("--base-imagery-url")?)?;
            }
            "--base-imagery-max-zoom" => {
                config.base_imagery_max_zoom =
                    zoom("--base-imagery-max-zoom", value("--base-imagery-max-zoom")?)?;
            }
            "--dem-url" => config.dem_url = template(value("--dem-url")?)?,
            "--style" => config.style = Some(PathBuf::from(value("--style")?)),
            "--cache-dir" => config.cache_dir = Some(PathBuf::from(value("--cache-dir")?)),
            "--calibration-dir" => {
                config.calibration_dir = Some(PathBuf::from(value("--calibration-dir")?));
            }
            "--fps" => config.fps = fps(value("--fps")?)?,
            "--fpv-size" => config.fpv_size = size("--fpv-size", value("--fpv-size")?)?,
            "--gimbal-size" => config.gimbal_size = size("--gimbal-size", value("--gimbal-size")?)?,
            "--prewarm" => config.prewarm = Some(site(value("--prewarm")?)?),
            other => return Err(ArgsError::Unknown(other.to_owned())),
        }
    }
    config.host_port = host_port.ok_or(ArgsError::NoHostPort)?;
    if config.imagery_min_zoom > config.imagery_max_zoom {
        return Err(invalid(
            "--imagery-min-zoom",
            &config.imagery_min_zoom.to_string(),
            "a zoom no deeper than --imagery-max-zoom",
        ));
    }
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
    let tiled = ["{z}", "{x}", "{y}"].iter().all(|key| value.contains(key));
    if tiled || value.contains("{bbox-epsg-3857}") {
        Ok(value.to_owned())
    } else {
        Err(invalid(
            "a tile URL",
            value,
            "a template with {z}, {x} and {y}, or with {bbox-epsg-3857}",
        ))
    }
}

fn zoom(option: &'static str, value: &str) -> Result<u8, ArgsError> {
    match value.parse::<u8>() {
        Ok(zoom) if zoom <= 22 => Ok(zoom),
        _ => Err(invalid(option, value, "a zoom from 0 to 22")),
    }
}

fn fps(value: &str) -> Result<f64, ArgsError> {
    match value.parse::<f64>() {
        Ok(fps) if fps.is_finite() && fps > 0.0 && fps <= 120.0 => Ok(fps),
        _ => Err(invalid("--fps", value, "a rate above 0 and at most 120")),
    }
}

fn site(value: &str) -> Result<(f64, f64), ArgsError> {
    let parsed = value.split_once(',').and_then(|(latitude, longitude)| {
        Some((
            latitude.trim().parse::<f64>().ok()?,
            longitude.trim().parse::<f64>().ok()?,
        ))
    });
    match parsed {
        Some((latitude, longitude))
            if (-85.0..=85.0).contains(&latitude) && (-180.0..=180.0).contains(&longitude) =>
        {
            Ok((latitude, longitude))
        }
        _ => Err(invalid(
            "--prewarm",
            value,
            "LATITUDE,LONGITUDE in degrees, latitude within 85 of the equator",
        )),
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
