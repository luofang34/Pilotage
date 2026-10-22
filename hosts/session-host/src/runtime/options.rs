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
//! - `PILOTAGE_MISSION_AREA`: presence enables the executor over an area
//!   survey instead of a route. The value is the polygon as
//!   `lat_deg,lon_deg;lat_deg,lon_deg;...` with at least three vertices.
//!   The survey height is `PILOTAGE_MISSION_CRUISE_HEIGHT`, 110 m when
//!   unset. Set one of the route and the area, not both.
//! - `PILOTAGE_MISSION_SURVEY_FOV_DEG`: the camera's across-track field
//!   of view in degrees for a nadir camera; unset keeps the
//!   [`pilotage_mission::SurveyRequest`] default.
//! - `PILOTAGE_MISSION_SURVEY_SIDELAP`: the footprint fraction adjacent
//!   lines share, `0` to `0.95`; unset keeps the request default.
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
//! - `PILOTAGE_MISSION_CRUISE_SPEED`: along-track cruise speed in meters per
//!   second. It also raises the mission's horizontal ceiling to that speed;
//!   the host then tightens the ceiling to the envelope the vehicle
//!   advertises. Unset keeps the [`pilotage_mission::MissionConfig`]
//!   default.
//! - `PILOTAGE_MISSION_OWNSHIP`: `simulation-truth` (the default) fuses a
//!   simulator's truth oracle; `vehicle-estimate` flies the vehicle's own
//!   exported estimate as a declared pass-through solution (ADR-0051).
//!   Under `vehicle-estimate` the anchor is the launch point: a height the
//!   vehicle measures from its launch point is read as height above the
//!   anchor.
//! - `PILOTAGE_MISSION_TARGET`: `simulator` (the default) or
//!   `real-vehicle`. A real vehicle requires `vehicle-estimate`.
//! - `PILOTAGE_SIM_CAMERA_PORT`: presence enables the simulator camera
//!   attachment on that loopback port. A world-model camera producer
//!   dials it, renders from the vehicle's simulator truth, and returns
//!   the session's video. Simulation builds only.

use std::path::PathBuf;

use chrono::NaiveDate;
use pilotage_mission::fixture::GeoPointDegrees;
use pilotage_mission::{ExecutionTarget, OwnshipSource, SurveyRequest};

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
    /// Bind address and transport admission credentials.
    pub network: super::NetworkOptions,
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
    /// The loopback port a simulator camera producer dials, enabled only
    /// when `PILOTAGE_SIM_CAMERA_PORT` is present. `None` attaches no
    /// simulator camera.
    pub sim_camera_port: Option<u16>,
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

/// The survey height when `PILOTAGE_MISSION_CRUISE_HEIGHT` is unset.
pub const SURVEY_DEFAULT_HEIGHT_M: f64 = 110.0;

/// What the mission executor flies.
#[derive(Debug, Clone, PartialEq)]
pub enum MissionPlanRequest {
    /// A route string expanded against the navdata snapshot at plan build.
    Route(String),
    /// An area survey planned at the cruise height above the anchor.
    Survey(SurveyRequest),
}

/// Configuration of the in-process mission executor.
#[derive(Debug, Clone, PartialEq)]
pub struct MissionOptions {
    /// The plan the executor flies.
    pub plan: MissionPlanRequest,
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
    /// Where ownship position comes from.
    pub ownship_source: OwnshipSource,
    /// The execution target the mission document is admitted for.
    pub execution_target: ExecutionTarget,
    /// Cruise speed override in meters per second; `None` keeps the
    /// [`pilotage_mission::MissionConfig`] default.
    pub cruise_mps: Option<f64>,
}

impl RuntimeOptions {
    pub(super) fn from_env() -> Result<Self, HostError> {
        let legacy_compatibility = matches!(
            std::env::var("PILOTAGE_LEGACY_COMPAT").as_deref(),
            Ok("1" | "true")
        );
        Ok(Self {
            network: super::NetworkOptions::default(),
            legacy_compatibility,
            mission: mission_from_env()?,
            sim_camera_port: env_var("PILOTAGE_SIM_CAMERA_PORT")?
                .map(|value| parse_sim_camera_port(&value))
                .transpose()?,
        })
    }
}

/// Parses `PILOTAGE_SIM_CAMERA_PORT`. Port 0 is refused: the producer must
/// know the port it dials, so an OS-chosen one would attach nothing.
fn parse_sim_camera_port(value: &str) -> Result<u16, HostError> {
    match value.parse::<u16>() {
        Ok(port) if port != 0 => Ok(port),
        _ => Err(HostError::SimCameraPort {
            value: value.to_owned(),
        }),
    }
}

/// Parses the `PILOTAGE_MISSION_*` family; `PILOTAGE_MISSION_ROUTE` being
/// unset disables the executor entirely.
fn mission_from_env() -> Result<Option<MissionOptions>, HostError> {
    let plan = match (
        env_var("PILOTAGE_MISSION_ROUTE")?,
        env_var("PILOTAGE_MISSION_AREA")?,
    ) {
        (None, None) => return Ok(None),
        (Some(_), Some(_)) => return Err(HostError::MissionPlanConflict),
        (Some(route), None) => MissionPlanRequest::Route(if route == "fixture" {
            pilotage_mission::fixture::DEMO_ROUTE.to_owned()
        } else {
            route
        }),
        (None, Some(area)) => MissionPlanRequest::Survey(survey_from_env(&area)?),
    };
    let navdata = match env_var("PILOTAGE_MISSION_NAVDATA")? {
        None => MissionNavdataSource::Fixture,
        Some(value) if value == "fixture" => MissionNavdataSource::Fixture,
        Some(path) => MissionNavdataSource::Store(PathBuf::from(path)),
    };
    let anchor = match env_var("PILOTAGE_MISSION_ANCHOR")? {
        None => DEFAULT_MISSION_ANCHOR,
        Some(value) => parse_anchor(&value)?,
    };
    let date =
        match env_var("PILOTAGE_MISSION_DATE")? {
            None => None,
            Some(value) => Some(NaiveDate::parse_from_str(&value, "%Y-%m-%d").map_err(
                |source| HostError::MissionDate {
                    value: value.clone(),
                    source,
                },
            )?),
        };
    if date.is_none()
        && matches!(navdata, MissionNavdataSource::Store(_))
        && matches!(plan, MissionPlanRequest::Route(_))
    {
        return Err(HostError::MissionDateMissing);
    }
    let cruise_height_m = match env_var("PILOTAGE_MISSION_CRUISE_HEIGHT")? {
        None => matches!(plan, MissionPlanRequest::Survey(_)).then_some(SURVEY_DEFAULT_HEIGHT_M),
        Some(value) => Some(parse_cruise_height(&value)?),
    };
    let cruise_mps = match env_var("PILOTAGE_MISSION_CRUISE_SPEED")? {
        None => None,
        Some(value) => Some(parse_cruise_speed(&value)?),
    };
    let ownship_source = match env_var("PILOTAGE_MISSION_OWNSHIP")? {
        None => OwnshipSource::SimulationTruth,
        Some(value) => parse_ownship_source(&value)?,
    };
    let execution_target = match env_var("PILOTAGE_MISSION_TARGET")? {
        None => ExecutionTarget::Simulator,
        Some(value) => parse_execution_target(&value)?,
    };
    check_vehicle_pairing(ownship_source, execution_target)?;
    Ok(Some(MissionOptions {
        plan,
        navdata,
        anchor,
        date,
        cruise_height_m,
        ownship_source,
        execution_target,
        cruise_mps,
    }))
}

/// Parses and range-screens `PILOTAGE_MISSION_CRUISE_SPEED`: a finite
/// positive speed. The vehicle's advertised envelope bounds it later.
fn parse_cruise_speed(value: &str) -> Result<f64, HostError> {
    let speed = value
        .parse::<f64>()
        .map_err(|source| HostError::MissionCruiseSpeed {
            value: value.to_owned(),
            source,
        })?;
    if !speed.is_finite() || speed <= 0.0 {
        return Err(HostError::MissionCruiseSpeedRange { speed });
    }
    Ok(speed)
}

/// The survey request from `PILOTAGE_MISSION_AREA` and the optional
/// camera parameters.
fn survey_from_env(area: &str) -> Result<SurveyRequest, HostError> {
    let mut request = SurveyRequest::new(parse_area(area)?);
    if let Some(value) = env_var("PILOTAGE_MISSION_SURVEY_FOV_DEG")? {
        request.across_track_fov_rad = parse_survey_fov_deg(&value)?.to_radians();
    }
    if let Some(value) = env_var("PILOTAGE_MISSION_SURVEY_SIDELAP")? {
        request.sidelap = parse_survey_sidelap(&value)?;
    }
    Ok(request)
}

/// Parses `lat_deg,lon_deg;lat_deg,lon_deg;...` into polygon vertices.
fn parse_area(value: &str) -> Result<Vec<[f64; 2]>, HostError> {
    let invalid = || HostError::MissionArea {
        value: value.to_owned(),
    };
    let vertices = value
        .split(';')
        .map(|pair| {
            let parts: Vec<&str> = pair.split(',').map(str::trim).collect();
            let [latitude, longitude] = parts.as_slice() else {
                return Err(invalid());
            };
            let parse = |part: &str| part.parse::<f64>().map_err(|_| invalid());
            Ok([parse(latitude)?, parse(longitude)?])
        })
        .collect::<Result<Vec<_>, _>>()?;
    if vertices.len() < 3 {
        return Err(invalid());
    }
    Ok(vertices)
}

/// Parses `PILOTAGE_MISSION_SURVEY_FOV_DEG`: a finite angle in (0, 180).
fn parse_survey_fov_deg(value: &str) -> Result<f64, HostError> {
    let invalid = |reason| HostError::MissionSurveyParameter {
        variable: "PILOTAGE_MISSION_SURVEY_FOV_DEG",
        value: value.to_owned(),
        reason,
    };
    let degrees = value
        .parse::<f64>()
        .map_err(|_| invalid("expected a number of degrees"))?;
    if !degrees.is_finite() || degrees <= 0.0 || degrees >= 180.0 {
        return Err(invalid("expected an angle between 0 and 180 degrees"));
    }
    Ok(degrees)
}

/// Parses `PILOTAGE_MISSION_SURVEY_SIDELAP`: a finite fraction in [0, 0.95].
fn parse_survey_sidelap(value: &str) -> Result<f64, HostError> {
    let invalid = |reason| HostError::MissionSurveyParameter {
        variable: "PILOTAGE_MISSION_SURVEY_SIDELAP",
        value: value.to_owned(),
        reason,
    };
    let sidelap = value
        .parse::<f64>()
        .map_err(|_| invalid("expected a fraction"))?;
    if !sidelap.is_finite() || !(0.0..=0.95).contains(&sidelap) {
        return Err(invalid("expected a fraction between 0 and 0.95"));
    }
    Ok(sidelap)
}

/// Parses `PILOTAGE_MISSION_OWNSHIP`; an unknown word fails startup.
fn parse_ownship_source(value: &str) -> Result<OwnshipSource, HostError> {
    match value {
        "simulation-truth" => Ok(OwnshipSource::SimulationTruth),
        "vehicle-estimate" => Ok(OwnshipSource::vehicle_estimate()),
        _ => Err(HostError::MissionOwnshipSource {
            value: value.to_owned(),
        }),
    }
}

/// Parses `PILOTAGE_MISSION_TARGET`; an unknown word fails startup.
fn parse_execution_target(value: &str) -> Result<ExecutionTarget, HostError> {
    match value {
        "simulator" => Ok(ExecutionTarget::Simulator),
        "real-vehicle" => Ok(ExecutionTarget::RealVehicle),
        _ => Err(HostError::MissionTarget {
            value: value.to_owned(),
        }),
    }
}

/// A real vehicle exports no simulation truth: that pairing never gets a
/// solution, so it is refused at startup instead of waiting in flight.
fn check_vehicle_pairing(source: OwnshipSource, target: ExecutionTarget) -> Result<(), HostError> {
    match (source, target) {
        (OwnshipSource::SimulationTruth, ExecutionTarget::RealVehicle) => {
            Err(HostError::MissionTruthOnRealVehicle)
        }
        _ => Ok(()),
    }
}

/// Parses and range-screens `PILOTAGE_MISSION_CRUISE_HEIGHT`. A negative
/// height would plan every waypoint below the launch elevation, and a
/// non-finite one disables every altitude comparison downstream; zero
/// legitimately disables the climb phase.
fn parse_cruise_height(value: &str) -> Result<f64, HostError> {
    let height = value
        .parse::<f64>()
        .map_err(|source| HostError::MissionCruiseHeight {
            value: value.to_owned(),
            source,
        })?;
    if !height.is_finite() || height < 0.0 {
        return Err(HostError::MissionCruiseHeightRange { height });
    }
    Ok(height)
}

/// Reads one host environment variable, treating "unset" as `None` and
/// a non-UTF-8 value as a startup error rather than a silent fallback.
fn env_var(variable: &'static str) -> Result<Option<String>, HostError> {
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

#[cfg(test)]
mod tests;
