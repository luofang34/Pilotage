//! Explicit serial and capture paths for the DJI bench adapter.

use std::ffi::OsString;
use std::path::PathBuf;

use pilotage_adapter_dji::{DjiAdapter, DjiAdapterError, DjiConfig, DjiControlMode};
use pilotage_protocol::VehicleId;

const SERIAL: &str = "PILOTAGE_DJI_SERIAL";
const CAPTURE: &str = "PILOTAGE_DJI_CAPTURE";
const CONTROL_MODE: &str = "PILOTAGE_DJI_CONTROL_MODE";

/// A DJI host configuration or startup failure.
#[derive(Debug, thiserror::Error)]
pub enum DjiHostError {
    /// An explicit path was not supplied.
    #[error("{variable} must name an explicit path")]
    Missing {
        /// The required environment variable.
        variable: &'static str,
    },
    /// The selected serial port is not a macOS USB CDC callout device.
    #[error("DJI serial path {path} must be a /dev/cu.usbmodem callout device")]
    SerialPath {
        /// The rejected path.
        path: PathBuf,
    },
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
    /// Opening the device or capture failed.
    #[error("cannot start the DJI bench adapter: {0}")]
    Adapter(#[source] DjiAdapterError),
}

pub(super) fn start_blocking(vehicle: VehicleId) -> Result<DjiAdapter, DjiHostError> {
    let config = config_from_values(
        std::env::var_os(SERIAL),
        std::env::var_os(CAPTURE),
        std::env::var_os(CONTROL_MODE),
    )?;
    tracing::info!(serial = %config.serial_path.display(), capture = %config.capture_path.display(),
        control_mode = ?config.control_mode,
        "DJI bench object: flight, positioning, and video are not implemented");
    if config.control_mode == DjiControlMode::BenchInitialHorizontal {
        tracing::warn!(
            "DJI one-way bench control starts from an operator-confirmed horizontal target; physical pose and stop are not confirmed"
        );
    }
    DjiAdapter::start_blocking(vehicle, config).map_err(DjiHostError::Adapter)
}

fn config_from_values(
    serial: Option<OsString>,
    capture: Option<OsString>,
    control_mode: Option<OsString>,
) -> Result<DjiConfig, DjiHostError> {
    let serial_path = required_path(serial, SERIAL)?;
    if serial_path.parent() != Some(std::path::Path::new("/dev"))
        || !serial_path.file_name().is_some_and(|name| {
            name.to_str().is_some_and(|name| {
                name.strip_prefix("cu.usbmodem")
                    .is_some_and(|suffix| !suffix.is_empty())
            })
        })
    {
        return Err(DjiHostError::SerialPath { path: serial_path });
    }
    let capture_path = required_path(capture, CAPTURE)?;
    if !capture_path.is_absolute()
        || capture_path.starts_with("/dev")
        || capture_path.file_name().is_none()
    {
        return Err(DjiHostError::CapturePath { path: capture_path });
    }
    Ok(DjiConfig {
        serial_path,
        capture_path,
        control_mode: parse_control_mode(control_mode)?,
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

fn required_path(value: Option<OsString>, variable: &'static str) -> Result<PathBuf, DjiHostError> {
    value
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or(DjiHostError::Missing { variable })
}

#[cfg(test)]
mod tests;
