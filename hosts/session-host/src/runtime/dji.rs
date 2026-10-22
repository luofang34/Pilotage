//! Explicit attachment, capture, and decoder settings for the DJI adapter.

use std::ffi::OsString;
use std::path::PathBuf;

use pilotage_adapter_api::RawVideoFrame;
use pilotage_adapter_dji::{DjiAdapterError, DjiControlMode};
use pilotage_protocol::VehicleId;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::dji_video::{self, DjiVideoError};

const SERIAL: &str = "PILOTAGE_DJI_SERIAL";
const USB_REGISTRY_ID: &str = "PILOTAGE_DJI_USB_REGISTRY_ID";
const USB: &str = "PILOTAGE_DJI_USB";
const GROUND_RECOVERY: &str = "PILOTAGE_DJI_USB_GROUND_RECOVERY";
const CAPTURE: &str = "PILOTAGE_DJI_CAPTURE";
const CONTROL_MODE: &str = "PILOTAGE_DJI_CONTROL_MODE";
const FFMPEG: &str = "PILOTAGE_DJI_FFMPEG";
const VIRTUAL_STICK: &str = "PILOTAGE_DJI_VIRTUAL_STICK";
const HEADING_REFERENCE: &str = "PILOTAGE_DJI_HEADING_REFERENCE";
const EXPERIMENTAL_FLIGHT: &str = "PILOTAGE_DJI_EXPERIMENTAL_FLIGHT";

mod attachment;
mod managed;
use attachment::{AttachmentSelection, parse_attachment};

/// A DJI host configuration or startup failure.
#[derive(Debug, thiserror::Error)]
pub enum DjiHostError {
    /// An explicit path was not supplied.
    #[error("{variable} must name an explicit path")]
    Missing {
        /// The required environment variable.
        variable: &'static str,
    },
    /// A ground-only USB recovery opt-in was not recognized.
    #[error("{GROUND_RECOVERY} must be absent or 1, got {value:?}")]
    GroundRecovery {
        /// The rejected setting.
        value: OsString,
    },
    /// The verified heading reference was not recognized.
    #[error("{HEADING_REFERENCE} must be absent, true or magnetic, got {value:?}")]
    HeadingReference {
        /// The rejected setting.
        value: OsString,
    },
    /// No attachment was selected.
    #[error("set exactly one of {SERIAL}, {USB_REGISTRY_ID}, or {USB}=auto")]
    MissingAttachment,
    /// More than one attachment setting was supplied.
    #[error("{SERIAL}, {USB_REGISTRY_ID}, and {USB} cannot be combined")]
    AttachmentConflict,
    /// The automatic USB selection setting is not supported.
    #[error("{USB} must be auto, got {value:?}")]
    UsbSelection {
        /// The rejected value.
        value: OsString,
    },
    /// The registry could not provide a complete candidate list.
    #[cfg(target_os = "macos")]
    #[error("cannot discover DJI upper USB candidates: {0}")]
    UsbDiscovery(#[source] pilotage_dji_usb::UsbError),
    /// Automatic selection found no matching host profile.
    #[cfg(target_os = "macos")]
    #[error(
        "no enumerated DJI upper USB profile was found; automatic selection does not change USB roles"
    )]
    UsbNotFound,
    /// Automatic selection found more than one matching host profile.
    #[cfg(target_os = "macos")]
    #[error(
        "multiple DJI upper USB candidates {registry_ids:?}; set {USB_REGISTRY_ID} to select one"
    )]
    UsbAmbiguous {
        /// The matching current attachment identities.
        registry_ids: Vec<u64>,
    },
    /// The selected serial port is not an explicit supported USB CDC device.
    #[error("DJI serial path {path} must name a USB CDC device under /dev")]
    SerialPath {
        /// The rejected path.
        path: PathBuf,
    },
    /// The USB attachment identity is not a nonzero integer.
    #[error("{USB_REGISTRY_ID} must be a nonzero decimal or 0x hexadecimal integer, got {value:?}")]
    RegistryId {
        /// The rejected value.
        value: OsString,
    },
    /// The USB attachment identity exceeds the accepted integer range.
    #[error("{USB_REGISTRY_ID} value {value:?} cannot be parsed: {source}")]
    RegistryIdRange {
        /// The rejected value.
        value: OsString,
        /// The integer conversion failure.
        #[source]
        source: std::num::ParseIntError,
    },
    /// The host cannot use the macOS USB attachment on this platform.
    #[error("DJI upper USB selection requires macOS; no serial fallback is selected")]
    UsbPlatform,
    /// The capture destination is not an absolute file path.
    #[error("DJI capture path {path} must be an absolute file path outside /dev")]
    CapturePath {
        /// The rejected path.
        path: PathBuf,
    },
    /// The requested control mode is not supported.
    #[error("{CONTROL_MODE} must be observed or bench-initial-horizontal, got {value:?}")]
    ControlMode {
        /// The rejected configuration value.
        value: OsString,
    },
    /// The flight opt-in is not the exact supported value.
    #[error("{EXPERIMENTAL_FLIGHT} must be absent or 1, got {value:?}")]
    ExperimentalFlight {
        /// The rejected configuration value.
        value: OsString,
    },
    /// The virtual-stick opt-in is not the exact supported value.
    #[error("{VIRTUAL_STICK} must be absent or 1, got {value:?}")]
    VirtualStick {
        /// The rejected configuration value.
        value: OsString,
    },
    /// The decoder command is empty or a relative path.
    #[error("{FFMPEG} must be a command name or an absolute file path, got {path:?}")]
    DecoderPath {
        /// The rejected path.
        path: PathBuf,
    },
    /// Opening the device or capture failed.
    #[error("cannot start the DJI adapter: {0}")]
    Adapter(#[source] DjiAdapterError),
    /// Starting or operating the video decoder failed.
    #[error("DJI video decoder: {0}")]
    Video(#[source] DjiVideoError),
    /// The video supervisor cannot accept a new connection.
    #[error("DJI video lifecycle is unavailable")]
    VideoLifecycle,
    /// The connection lifecycle task could not start.
    #[error("cannot start DJI connection lifecycle task: {0}")]
    LifecycleSpawn(#[source] std::io::Error),
}

pub(super) struct DjiRuntime {
    pub(super) adapter: managed::ManagedDji,
    pub(super) frames: Option<mpsc::Receiver<RawVideoFrame>>,
    pub(super) producer: Option<JoinHandle<()>>,
}

struct DjiHostConfig {
    attachment: AttachmentSelection,
    capture_path: PathBuf,
    control_mode: DjiControlMode,
    ffmpeg: PathBuf,
    experimental_flight: bool,
    usb_ground_recovery: bool,
    /// The north reference the operator verified for the reported heading.
    heading_reference: Option<pilotage_protocol::HeadingReference>,
}

pub(super) fn start_blocking(
    vehicle: VehicleId,
    origin: std::time::Instant,
) -> Result<DjiRuntime, DjiHostError> {
    let mut config = config_from_values(
        std::env::var_os(SERIAL),
        std::env::var_os(USB_REGISTRY_ID),
        std::env::var_os(USB),
        std::env::var_os(CAPTURE),
        std::env::var_os(CONTROL_MODE),
        std::env::var_os(FFMPEG),
        std::env::var_os(EXPERIMENTAL_FLIGHT),
    )?;
    config.usb_ground_recovery = match std::env::var_os(GROUND_RECOVERY) {
        None => false,
        Some(value) if value == "1" => true,
        Some(value) => return Err(DjiHostError::GroundRecovery { value }),
    };
    config.heading_reference = parse_heading_reference(std::env::var_os(HEADING_REFERENCE))?;
    let virtual_stick = parse_virtual_stick(std::env::var_os(VIRTUAL_STICK))?;
    let (streams, updates) = mpsc::channel(2);
    let (frames, producer) = dji_video::supervise(config.ffmpeg.clone(), updates);
    match managed::ManagedDji::start_blocking(vehicle, config, virtual_stick, streams, origin) {
        Ok(adapter) => Ok(DjiRuntime {
            adapter,
            frames: Some(frames),
            producer: Some(producer),
        }),
        Err(error) => {
            producer.abort();
            Err(error)
        }
    }
}

fn config_from_values(
    serial: Option<OsString>,
    usb_registry_id: Option<OsString>,
    usb: Option<OsString>,
    capture: Option<OsString>,
    control_mode: Option<OsString>,
    ffmpeg: Option<OsString>,
    experimental_flight: Option<OsString>,
) -> Result<DjiHostConfig, DjiHostError> {
    let attachment = parse_attachment(serial, usb_registry_id, usb)?;
    let capture_path = required_path(capture, CAPTURE)?;
    if !capture_path.is_absolute()
        || capture_path.starts_with("/dev")
        || capture_path.file_name().is_none()
    {
        return Err(DjiHostError::CapturePath { path: capture_path });
    }
    Ok(DjiHostConfig {
        attachment,
        capture_path,
        control_mode: parse_control_mode(control_mode)?,
        ffmpeg: parse_decoder(ffmpeg)?,
        experimental_flight: parse_experimental_flight(experimental_flight)?,
        usb_ground_recovery: false,
        heading_reference: None,
    })
}

/// The heading reference the operator verified on the ground: `true` or
/// `magnetic`. Anything else, including an empty value, is refused.
fn parse_heading_reference(
    value: Option<OsString>,
) -> Result<Option<pilotage_protocol::HeadingReference>, DjiHostError> {
    match value {
        None => Ok(None),
        Some(value) if value == "true" => Ok(Some(pilotage_protocol::HeadingReference::True)),
        Some(value) if value == "magnetic" => {
            Ok(Some(pilotage_protocol::HeadingReference::Magnetic))
        }
        Some(value) => Err(DjiHostError::HeadingReference { value }),
    }
}

fn parse_virtual_stick(value: Option<OsString>) -> Result<bool, DjiHostError> {
    match value {
        None => Ok(false),
        Some(value) if value == "1" => Ok(true),
        Some(value) => Err(DjiHostError::VirtualStick { value }),
    }
}

fn parse_experimental_flight(value: Option<OsString>) -> Result<bool, DjiHostError> {
    match value {
        None => Ok(false),
        Some(value) if value == "1" => Ok(true),
        Some(value) => Err(DjiHostError::ExperimentalFlight { value }),
    }
}

fn parse_control_mode(value: Option<OsString>) -> Result<DjiControlMode, DjiHostError> {
    match value {
        None => Ok(DjiControlMode::Observed),
        Some(value) => match value.to_str() {
            Some("observed") => Ok(DjiControlMode::Observed),
            Some("bench-initial-horizontal") => Ok(DjiControlMode::BenchInitialHorizontal),
            _ => Err(DjiHostError::ControlMode { value }),
        },
    }
}

fn parse_decoder(value: Option<OsString>) -> Result<PathBuf, DjiHostError> {
    let path = value.map_or_else(|| PathBuf::from("ffmpeg"), PathBuf::from);
    if path.as_os_str().is_empty()
        || path.file_name().is_none()
        || (!path.is_absolute() && path.file_name() != Some(path.as_os_str()))
    {
        return Err(DjiHostError::DecoderPath { path });
    }
    Ok(path)
}

fn required_path(value: Option<OsString>, variable: &'static str) -> Result<PathBuf, DjiHostError> {
    value
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or(DjiHostError::Missing { variable })
}

#[cfg(test)]
mod tests;
