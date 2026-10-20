//! Explicit attachment, capture, and decoder settings for the DJI adapter.

use std::ffi::OsString;
use std::path::PathBuf;

use pilotage_adapter_api::RawVideoFrame;
use pilotage_adapter_dji::{DjiAdapter, DjiAdapterError, DjiConfig, DjiControlMode};
use pilotage_protocol::VehicleId;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::dji_video::{self, DjiVideoConfig, DjiVideoError};

const SERIAL: &str = "PILOTAGE_DJI_SERIAL";
const USB_REGISTRY_ID: &str = "PILOTAGE_DJI_USB_REGISTRY_ID";
const USB: &str = "PILOTAGE_DJI_USB";
const CAPTURE: &str = "PILOTAGE_DJI_CAPTURE";
const CONTROL_MODE: &str = "PILOTAGE_DJI_CONTROL_MODE";
const FFMPEG: &str = "PILOTAGE_DJI_FFMPEG";

mod attachment;
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
}

pub(super) struct DjiRuntime {
    pub(super) adapter: DjiAdapter,
    pub(super) frames: Option<mpsc::Receiver<RawVideoFrame>>,
    pub(super) producer: Option<JoinHandle<()>>,
}

struct DjiHostConfig {
    attachment: AttachmentSelection,
    capture_path: PathBuf,
    control_mode: DjiControlMode,
    ffmpeg: PathBuf,
}

pub(super) fn start_blocking(vehicle: VehicleId) -> Result<DjiRuntime, DjiHostError> {
    let config = config_from_values(
        std::env::var_os(SERIAL),
        std::env::var_os(USB_REGISTRY_ID),
        std::env::var_os(USB),
        std::env::var_os(CAPTURE),
        std::env::var_os(CONTROL_MODE),
        std::env::var_os(FFMPEG),
    )?;
    let adapter_config = DjiConfig {
        attachment: config.attachment.resolve_blocking()?,
        capture_path: config.capture_path,
        control_mode: config.control_mode,
    };
    tracing::info!(attachment = ?adapter_config.attachment,
        capture = %adapter_config.capture_path.display(),
        control_mode = ?adapter_config.control_mode,
        "DJI bench object: flight and positioning control are not implemented");
    if adapter_config.control_mode == DjiControlMode::BenchInitialHorizontal {
        tracing::warn!(
            "DJI one-way bench control starts from an operator-confirmed horizontal target; physical pose and stop are not confirmed"
        );
    }
    let mut adapter =
        DjiAdapter::start_blocking(vehicle, adapter_config).map_err(DjiHostError::Adapter)?;
    let video = adapter
        .subscribe_encoded_video()
        .map(|stream| {
            dji_video::spawn(
                DjiVideoConfig {
                    ffmpeg: config.ffmpeg,
                    incarnation: stream.incarnation,
                    origin: tokio::time::Instant::from_std(stream.origin),
                    keyframe_request: stream.keyframe_request,
                },
                stream.receiver,
            )
        })
        .transpose()
        .map_err(DjiHostError::Video)?;
    let (frames, producer) = match video {
        Some((frames, producer)) => (Some(frames), Some(producer)),
        None => (None, None),
    };
    Ok(DjiRuntime {
        adapter,
        frames,
        producer,
    })
}

fn config_from_values(
    serial: Option<OsString>,
    usb_registry_id: Option<OsString>,
    usb: Option<OsString>,
    capture: Option<OsString>,
    control_mode: Option<OsString>,
    ffmpeg: Option<OsString>,
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
    })
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
