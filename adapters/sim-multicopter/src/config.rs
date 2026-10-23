//! Configuration of one simulated multicopter.

use std::path::PathBuf;

use crate::camera::fault::CameraFault;
use crate::geodesy::LaunchPoint;

/// The Esri World Imagery tile template, `{z}/{y}/{x}` order.
pub const ESRI_WORLD_IMAGERY: &str =
    "https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}";

/// The flight envelope and the response of the point-mass model.
///
/// The defaults are the DJI virtual-stick control range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    /// Horizontal speed limit, metres per second.
    pub horizontal_mps: f64,
    /// Vertical speed limit, metres per second.
    pub vertical_mps: f64,
    /// Yaw rate limit, radians per second.
    pub yaw_rate_rps: f64,
    /// First-order time constant of the velocity response, seconds.
    pub velocity_time_constant_s: f64,
    /// First-order time constant of the yaw-rate response, seconds.
    pub yaw_time_constant_s: f64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            horizontal_mps: 15.0,
            vertical_mps: 4.0,
            yaw_rate_rps: 100.0_f64.to_radians(),
            velocity_time_constant_s: 0.5,
            yaw_time_constant_s: 0.5,
        }
    }
}

/// The battery energy model.
///
/// The defaults approximate a DJI Mini 3 two-cell pack.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BatteryConfig {
    /// Usable energy of a full pack, watt-hours.
    pub capacity_wh: f64,
    /// Electrical power in a hover, watts.
    pub hover_power_w: f64,
    /// Electrical power at [`Self::cruise_speed_mps`], watts.
    pub cruise_power_w: f64,
    /// The ground speed at which the model draws [`Self::cruise_power_w`].
    pub cruise_speed_mps: f64,
    /// Electrical power with the motors at idle on the ground, watts.
    pub idle_power_w: f64,
    /// Pack voltage when full, volts.
    pub full_voltage_v: f64,
    /// Pack voltage when empty, volts.
    pub empty_voltage_v: f64,
}

impl Default for BatteryConfig {
    fn default() -> Self {
        Self {
            capacity_wh: 18.1,
            hover_power_w: 28.0,
            cruise_power_w: 36.0,
            cruise_speed_mps: 10.0,
            idle_power_w: 6.0,
            full_voltage_v: 8.4,
            empty_voltage_v: 6.8,
        }
    }
}

/// Where the camera gets its ground imagery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TileSourceConfig {
    /// Web-mercator tiles over HTTP. The template contains `{z}`, `{x}`,
    /// and `{y}`.
    Http {
        /// The URL template.
        template: String,
        /// The directory that keeps downloaded tiles across runs.
        cache_dir: Option<PathBuf>,
    },
    /// A deterministic procedural texture; no network access.
    Procedural,
}

/// The gimbal camera.
#[derive(Debug, Clone, PartialEq)]
pub struct CameraConfig {
    /// Frame width, pixels.
    pub width: u32,
    /// Frame height, pixels.
    pub height: u32,
    /// Frames per second of simulation time.
    pub fps: u32,
    /// Diagonal field of view, degrees.
    pub diagonal_fov_deg: f64,
    /// The ground imagery.
    pub tiles: TileSourceConfig,
    /// The highest tile zoom level the camera asks for.
    pub max_zoom: u8,
    /// Injected image faults.
    pub faults: Vec<CameraFault>,
    /// A directory that receives every 25th rendered frame as a PNG file.
    pub frame_dump: Option<PathBuf>,
}

impl Default for CameraConfig {
    fn default() -> Self {
        Self {
            width: 640,
            height: 360,
            fps: 5,
            diagonal_fov_deg: 82.1,
            tiles: TileSourceConfig::Http {
                template: ESRI_WORLD_IMAGERY.to_owned(),
                cache_dir: None,
            },
            max_zoom: 19,
            faults: Vec::new(),
            frame_dump: None,
        }
    }
}

/// Everything one simulated vehicle needs.
#[derive(Debug, Clone, PartialEq)]
pub struct SimConfig {
    /// The launch point; the simulated ground is flat at its altitude.
    pub launch: LaunchPoint,
    /// The display name suffix; the display name always starts with `SIM`.
    pub name: String,
    /// The initial heading, degrees from true north.
    pub initial_heading_deg: f64,
    /// The initial gimbal pitch, degrees; negative looks down.
    pub initial_gimbal_pitch_deg: f64,
    /// The flight envelope.
    pub limits: Limits,
    /// The battery model.
    pub battery: BatteryConfig,
    /// The gimbal camera.
    pub camera: CameraConfig,
    /// The seed of the procedural texture.
    pub seed: u64,
}

impl SimConfig {
    /// The defaults at `launch`, with the camera looking straight down
    /// for an area survey.
    #[must_use]
    pub fn new(launch: LaunchPoint) -> Self {
        Self {
            launch,
            name: String::from("multicopter"),
            initial_heading_deg: 0.0,
            initial_gimbal_pitch_deg: -90.0,
            limits: Limits::default(),
            battery: BatteryConfig::default(),
            camera: CameraConfig::default(),
            seed: 0,
        }
    }

    /// The vehicle's display name: `SIM` and the configured name.
    #[must_use]
    pub fn display_name(&self) -> String {
        format!("SIM {}", self.name)
    }
}
