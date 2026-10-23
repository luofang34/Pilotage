//! The objective file: a survey in plain JSON.
//!
//! ```json
//! {"title": "Battlefield to Nassau Hall", "area": [[40.33, -74.67], ...],
//!  "launch": [40.33, -74.67, 40.0], "height_m": 110, "cruise_mps": 12,
//!  "sidelap": 0.6, "fov_deg": 69.6, "min_usable_frames_per_line": 2,
//!  "line_bearing_deg": 0}
//! ```

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use pilotage_protocol::wire;
use serde::Deserialize;

use crate::cli::OperatorError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    title: String,
    area: Vec<[f64; 2]>,
    launch: [f64; 3],
    #[serde(default)]
    height_m: f64,
    #[serde(default)]
    cruise_mps: f64,
    #[serde(default)]
    sidelap: f64,
    #[serde(default)]
    fov_deg: f64,
    #[serde(default)]
    min_usable_frames_per_line: u32,
    #[serde(default)]
    line_bearing_deg: Option<f64>,
}

pub(crate) fn read_blocking(path: &Path) -> Result<wire::MissionObjective, OperatorError> {
    let error = |reason: String| OperatorError::Objective {
        path: path.to_path_buf(),
        reason,
    };
    let text = std::fs::read_to_string(path).map_err(|e| error(e.to_string()))?;
    let file: File = serde_json::from_str(&text).map_err(|e| error(e.to_string()))?;
    let point = |[latitude_deg, longitude_deg]: [f64; 2]| wire::GeoPoint {
        latitude_deg,
        longitude_deg,
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    Ok(wire::MissionObjective {
        objective_id: format!("objective-{now}"),
        origin: Some(wire::RecordOrigin {
            issuer: "local".to_owned(),
            subject: "mission-operator".to_owned(),
            organization: String::new(),
        }),
        revision: 1,
        previous_digest: Vec::new(),
        title: file.title,
        kind: Some(wire::mission_objective::Kind::Survey(
            wire::SurveyObjective {
                area: file.area.into_iter().map(point).collect(),
                launch: Some(point([file.launch[0], file.launch[1]])),
                launch_altitude_m: file.launch[2],
                height_m: file.height_m,
                cruise_mps: file.cruise_mps,
                sidelap: file.sidelap,
                across_track_fov_deg: file.fov_deg,
                return_to_launch: true,
                min_usable_frames_per_line: file.min_usable_frames_per_line,
                line_bearing_deg: file.line_bearing_deg,
            },
        )),
    })
}
