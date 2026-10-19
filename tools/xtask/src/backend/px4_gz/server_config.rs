//! The gz server configuration a PX4 session runs with: PX4's own, with
//! its Magnetometer system set to report a physically correct field.
//!
//! By default gz's Magnetometer system computes the field in NED, places
//! it on the ENU world axes unconverted, and reports gauss. That field is
//! mirrored east for west, so PX4's heading is off by twice the local
//! magnetic declination (about 25° at N51). PX4's gz bridge reads the
//! corrected output (tesla, sensor FLU) when the system runs with
//! `use_units_gauss` and `use_earth_frame_ned` off. This module turns both
//! off in a copy of PX4's configuration, so the PX4 checkout stays as it is.

use std::path::{Path, PathBuf};

use crate::error::XtaskError;

/// The copy's file name inside the session's run directory.
const CORRECTED_NAME: &str = "server.config";
/// The Magnetometer system's opening tag, up to its attributes' end.
const MAGNETOMETER: &str = "name=\"gz::sim::systems::Magnetometer\"";
/// The options that select the corrected output.
const CORRECTED_OPTIONS: &str =
    "<use_units_gauss>false</use_units_gauss><use_earth_frame_ned>false</use_earth_frame_ned>";

/// `config` with its single Magnetometer system set to the corrected
/// output. A configuration that already states either option is kept as
/// it is: its author chose.
///
/// # Errors
///
/// Returns the reason when the configuration has no single Magnetometer
/// system: PX4 would then fly with no magnetometer or two.
pub(crate) fn corrected(config: &str) -> Result<String, &'static str> {
    if config.matches(MAGNETOMETER).count() != 1 {
        return Err("no single gz::sim::systems::Magnetometer system");
    }
    if config.contains("use_units_gauss") || config.contains("use_earth_frame_ned") {
        return Ok(config.to_owned());
    }
    let at = config.find(MAGNETOMETER).ok_or("no Magnetometer system")?;
    let close = at
        + config[at..]
            .find('>')
            .ok_or("an unterminated Magnetometer system")?;
    let (head, tail) = config.split_at(close + 1);
    if let Some(open) = head.strip_suffix("/>") {
        Ok(format!(
            "{}>{CORRECTED_OPTIONS}</plugin>{tail}",
            open.trim_end()
        ))
    } else {
        Ok(format!("{head}{CORRECTED_OPTIONS}{tail}"))
    }
}

/// Writes the corrected copy of `source` into `run_dir` and returns its
/// path.
///
/// # Errors
///
/// Returns [`XtaskError::ServerConfig`] when `source` has no single
/// Magnetometer system, and [`XtaskError::Io`] when it cannot be read or
/// the copy cannot be written.
pub(crate) fn write_corrected(source: &Path, run_dir: &Path) -> Result<PathBuf, XtaskError> {
    let config = std::fs::read_to_string(source).map_err(|source| XtaskError::Io {
        context: "reading PX4's gz server config",
        source,
    })?;
    let corrected = corrected(&config).map_err(|reason| XtaskError::ServerConfig {
        path: source.to_path_buf(),
        reason,
    })?;
    std::fs::create_dir_all(run_dir).map_err(|source| XtaskError::Io {
        context: "creating the session run directory",
        source,
    })?;
    let target = run_dir.join(CORRECTED_NAME);
    std::fs::write(&target, corrected).map_err(|source| XtaskError::Io {
        context: "writing the corrected gz server config",
        source,
    })?;
    Ok(target)
}

#[cfg(test)]
mod tests;
