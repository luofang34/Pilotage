//! Explicit serial and capture paths for the DJI bench adapter.

use std::ffi::OsString;
use std::path::PathBuf;

use pilotage_adapter_dji::{DjiAdapter, DjiAdapterError, DjiConfig};
use pilotage_protocol::VehicleId;

const SERIAL: &str = "PILOTAGE_DJI_SERIAL";
const CAPTURE: &str = "PILOTAGE_DJI_CAPTURE";

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
    /// Opening the device or capture failed.
    #[error("cannot start the DJI bench adapter: {0}")]
    Adapter(#[source] DjiAdapterError),
}

pub(super) fn start_blocking(vehicle: VehicleId) -> Result<DjiAdapter, DjiHostError> {
    let config = config_from_values(std::env::var_os(SERIAL), std::env::var_os(CAPTURE))?;
    tracing::info!(serial = %config.serial_path.display(), capture = %config.capture_path.display(),
        "DJI bench object: flight and positioning are unavailable; gimbal requires live observations");
    DjiAdapter::start_blocking(vehicle, config).map_err(DjiHostError::Adapter)
}

fn config_from_values(
    serial: Option<OsString>,
    capture: Option<OsString>,
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
    })
}

fn required_path(value: Option<OsString>, variable: &'static str) -> Result<PathBuf, DjiHostError> {
    value
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or(DjiHostError::Missing { variable })
}

#[cfg(test)]
mod tests;
