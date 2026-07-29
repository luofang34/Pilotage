//! Runtime policy switches for a host instance, parsed from the process
//! environment at startup.
//!
//! Environment variables:
//!
//! - `PILOTAGE_LEGACY_COMPAT`: `1`/`true` admits legacy numeric payload
//!   frames (SIMULATION compatibility). Production control is typed-only.
//! - `PILOTAGE_MISSION_ROUTE`: presence enables the in-process mission
//!   executor (ADR-0025 automation principal). The value is the route
//!   string expanded against the navdata snapshot; the literal `fixture`
//!   selects [`pilotage_mission::fixture::DEMO_ROUTE`].
//! - `PILOTAGE_MISSION_NAVDATA`: `fixture` (the default) builds the demo
//!   snapshot and round-trips it through the real blob container; any
//!   other value is a navdata store directory whose `*/*.acnav` blobs are
//!   scanned per ADR-0030.
//! - `PILOTAGE_MISSION_ANCHOR`: mission anchor as `lat_deg,lon_deg,alt_m`
//!   (default `47.397742,8.545594,488.0`).
//! - `PILOTAGE_MISSION_DATE`: `YYYY-MM-DD` flight date used to select a
//!   covering cycle from a store. Required for a store; ignored (with a
//!   log line) for the fixture, whose cycle is fixed.
//! - `PILOTAGE_MISSION_CRUISE_HEIGHT`: cruise height above the anchor in
//!   meters. `0` skips the climb phase (planar vehicles); unset keeps the
//!   [`pilotage_mission::MissionConfig`] default.

use std::path::PathBuf;

use chrono::NaiveDate;
use pilotage_mission::fixture::GeoPointDegrees;

use crate::error::HostError;

/// The anchor used when `PILOTAGE_MISSION_ANCHOR` is unset.
pub const DEFAULT_MISSION_ANCHOR: GeoPointDegrees = GeoPointDegrees {
    lat_deg: 47.397742,
    lon_deg: 8.545594,
    alt_m: 488.0,
};

/// Runtime policy switches for a host instance.
///
/// [`super::start`] derives these from the process environment; tests inject
/// them directly through [`super::start_with_options`].
#[derive(Debug, Clone, Default)]
pub struct RuntimeOptions {
    /// SIMULATION compatibility: admit numeric legacy payload frames at the
    /// gate's translation boundary. Production control is TYPED-ONLY —
    /// legacy payloads bypass profile-activation binding and carry
    /// uncorrelated edges — so this never rides by default;
    /// [`super::start`] turns it on only for the explicit
    /// `PILOTAGE_LEGACY_COMPAT=1` opt-in.
    pub legacy_compatibility: bool,
    /// The in-process mission executor, enabled only when
    /// `PILOTAGE_MISSION_ROUTE` is present. `None` leaves every existing
    /// host behavior untouched: no automation task, no extra logs.
    pub mission: Option<MissionOptions>,
}

/// Where the mission executor's navdata snapshot comes from (ADR-0030).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissionNavdataSource {
    /// The generated demo snapshot, packed and decoded through the same
    /// blob container published data travels.
    Fixture,
    /// A store directory scanned for `*/*.acnav` blobs; the newest
    /// `faa-nasr` cycle covering the mission date wins.
    Store(PathBuf),
}

/// Configuration of the in-process mission executor.
#[derive(Debug, Clone, PartialEq)]
pub struct MissionOptions {
    /// Route string expanded against the snapshot at plan build.
    pub route: String,
    /// Snapshot source: fixture or a scanned store directory.
    pub navdata: MissionNavdataSource,
    /// Mission anchor, in the degree convention navdata uses; converted to
    /// radians exactly once at plan build (ADR-0030).
    pub anchor: GeoPointDegrees,
    /// Flight date used to select a covering cycle from a store; `None`
    /// only for the fixture, whose cycle is fixed.
    pub date: Option<NaiveDate>,
    /// Cruise height override in meters (`0.0` skips the climb phase);
    /// `None` keeps the [`pilotage_mission::MissionConfig`] default.
    pub cruise_height_m: Option<f64>,
}

impl RuntimeOptions {
    pub(super) fn from_env() -> Result<Self, HostError> {
        let legacy_compatibility = matches!(
            std::env::var("PILOTAGE_LEGACY_COMPAT").as_deref(),
            Ok("1" | "true")
        );
        Ok(Self {
            legacy_compatibility,
            mission: mission_from_env()?,
        })
    }
}

/// Parses the `PILOTAGE_MISSION_*` family; `PILOTAGE_MISSION_ROUTE` being
/// unset disables the executor entirely.
fn mission_from_env() -> Result<Option<MissionOptions>, HostError> {
    let Some(route) = mission_var("PILOTAGE_MISSION_ROUTE")? else {
        return Ok(None);
    };
    let route = if route == "fixture" {
        pilotage_mission::fixture::DEMO_ROUTE.to_owned()
    } else {
        route
    };
    let navdata = match mission_var("PILOTAGE_MISSION_NAVDATA")? {
        None => MissionNavdataSource::Fixture,
        Some(value) if value == "fixture" => MissionNavdataSource::Fixture,
        Some(path) => MissionNavdataSource::Store(PathBuf::from(path)),
    };
    let anchor = match mission_var("PILOTAGE_MISSION_ANCHOR")? {
        None => DEFAULT_MISSION_ANCHOR,
        Some(value) => parse_anchor(&value)?,
    };
    let date =
        match mission_var("PILOTAGE_MISSION_DATE")? {
            None => None,
            Some(value) => Some(NaiveDate::parse_from_str(&value, "%Y-%m-%d").map_err(
                |source| HostError::MissionDate {
                    value: value.clone(),
                    source,
                },
            )?),
        };
    if date.is_none() && matches!(navdata, MissionNavdataSource::Store(_)) {
        return Err(HostError::MissionDateMissing);
    }
    let cruise_height_m = match mission_var("PILOTAGE_MISSION_CRUISE_HEIGHT")? {
        None => None,
        Some(value) => {
            Some(
                value
                    .parse::<f64>()
                    .map_err(|source| HostError::MissionCruiseHeight {
                        value: value.clone(),
                        source,
                    })?,
            )
        }
    };
    Ok(Some(MissionOptions {
        route,
        navdata,
        anchor,
        date,
        cruise_height_m,
    }))
}

/// Reads one `PILOTAGE_MISSION_*` variable, treating "unset" as `None` and
/// a non-UTF-8 value as a startup error rather than a silent fallback.
fn mission_var(variable: &'static str) -> Result<Option<String>, HostError> {
    match std::env::var(variable) {
        Ok(value) => Ok(Some(value)),
        Err(std::env::VarError::NotPresent) => Ok(None),
        Err(source @ std::env::VarError::NotUnicode(_)) => {
            Err(HostError::MissionVarEncoding { variable, source })
        }
    }
}

/// Parses `lat_deg,lon_deg,alt_m` into the fixture's degree convention.
fn parse_anchor(value: &str) -> Result<GeoPointDegrees, HostError> {
    let invalid = || HostError::MissionAnchor {
        value: value.to_owned(),
    };
    let parts: Vec<&str> = value.split(',').map(str::trim).collect();
    let [lat, lon, alt] = parts.as_slice() else {
        return Err(invalid());
    };
    let parse = |part: &str| part.parse::<f64>().map_err(|_| invalid());
    Ok(GeoPointDegrees {
        lat_deg: parse(lat)?,
        lon_deg: parse(lon)?,
        alt_m: parse(alt)?,
    })
}
