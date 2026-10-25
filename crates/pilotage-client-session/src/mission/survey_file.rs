//! The survey objective as an operator or an agent writes it, in plain
//! JSON:
//!
//! ```json
//! {"title": "Battlefield to Nassau Hall", "area": [[40.33, -74.67], ...],
//!  "launch": [40.33, -74.67, 40.0], "height_m": 110, "cruise_mps": 12,
//!  "sidelap": 0.6, "fov_deg": 69.6, "min_usable_frames_per_line": 2,
//!  "line_bearing_deg": 0}
//! ```
//!
//! A zero or absent numeric field takes the lane's default.

use pilotage_protocol::wire;
use serde::{Deserialize, Serialize};

/// A survey objective in its JSON form.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SurveyFile {
    /// The objective's title.
    pub title: String,
    /// The area, latitude and longitude in degrees, in order.
    pub area: Vec<[f64; 2]>,
    /// The launch point's latitude, longitude and altitude.
    pub launch: [f64; 3],
    /// Height above the launch point, metres.
    #[serde(default)]
    pub height_m: f64,
    /// Cruise speed, metres per second.
    #[serde(default)]
    pub cruise_mps: f64,
    /// Fraction of a picture's width that adjacent lines share.
    #[serde(default)]
    pub sidelap: f64,
    /// Across-track field of view of the camera, degrees.
    #[serde(default)]
    pub fov_deg: f64,
    /// Usable pictures a line needs to count as done.
    #[serde(default)]
    pub min_usable_frames_per_line: u32,
    /// Bearing of the lines, degrees true; absent lets the lane choose.
    #[serde(default)]
    pub line_bearing_deg: Option<f64>,
    /// The operator requires `height_m`: a revision that changes it changes
    /// the objective.
    #[serde(default)]
    pub hold_height: bool,
    /// The area's identity and revision, when it is a named object.
    #[serde(default)]
    pub area_ref: Option<Box<AreaRef>>,
}

/// The identity and the revision of an area, and what it was found as.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AreaRef {
    /// The area's identity, stable across its revisions.
    pub id: String,
    /// The revision of its geometry; zero for an area whose vertices are
    /// all it is.
    #[serde(default)]
    pub revision: u64,
    /// What the area was found as, for display.
    #[serde(default)]
    pub source: String,
}

/// An area as the operator's tools find it: its identity, its revision and
/// its geometry, one object for every client that plans over it.
#[derive(Clone, Debug, PartialEq)]
pub struct Area {
    /// The area's identity and revision.
    pub identity: AreaRef,
    /// The area, latitude and longitude in degrees, in order around it.
    pub ring: Vec<[f64; 2]>,
}

impl Area {
    /// An area drawn as the object `id` in its `revision`.
    #[must_use]
    pub fn drawn(id: &str, revision: u64, source: &str, ring: Vec<[f64; 2]>) -> Self {
        Self {
            identity: AreaRef {
                id: id.to_owned(),
                revision,
                source: source.to_owned(),
            },
            ring,
        }
    }

    /// An area derived from the place `name`, such as labels on a map: it
    /// has no revision of its own, so its vertices are all it is.
    #[must_use]
    pub fn derived(name: &str, source: &str, ring: Vec<[f64; 2]>) -> Self {
        Self::drawn(&format!("derived:{name}"), 0, source, ring)
    }
}

impl SurveyFile {
    /// The objective `text` holds.
    ///
    /// # Errors
    ///
    /// Returns the JSON error for text that is not a survey objective.
    pub fn parse(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// A fresh first revision of the objective, issued by `subject` at
    /// `now_ms`, milliseconds since the Unix epoch.
    #[must_use]
    pub fn objective(&self, subject: &str, now_ms: u64) -> wire::MissionObjective {
        let point = |[latitude_deg, longitude_deg]: [f64; 2]| wire::GeoPoint {
            latitude_deg,
            longitude_deg,
        };
        let [launch_lat, launch_lon, launch_altitude_m] = self.launch;
        wire::MissionObjective {
            objective_id: format!("objective-{now_ms}"),
            origin: Some(wire::RecordOrigin {
                issuer: "local".to_owned(),
                subject: subject.to_owned(),
                organization: String::new(),
            }),
            revision: 1,
            previous_digest: Vec::new(),
            title: self.title.clone(),
            kind: Some(wire::mission_objective::Kind::Survey(
                wire::SurveyObjective {
                    area: self.area.iter().copied().map(point).collect(),
                    launch: Some(point([launch_lat, launch_lon])),
                    launch_altitude_m,
                    height_m: self.height_m,
                    cruise_mps: self.cruise_mps,
                    sidelap: self.sidelap,
                    across_track_fov_deg: self.fov_deg,
                    return_to_launch: true,
                    min_usable_frames_per_line: self.min_usable_frames_per_line,
                    line_bearing_deg: self.line_bearing_deg,
                    hold_height: self.hold_height,
                    area_ref: self.area_ref.as_ref().map(|r| wire::MissionAreaRef {
                        area_id: r.id.clone(),
                        revision: r.revision,
                        source: r.source.clone(),
                    }),
                },
            )),
            delegated_by: String::new(),
        }
    }
}
