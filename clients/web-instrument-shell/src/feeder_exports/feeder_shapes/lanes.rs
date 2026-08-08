//! Marshalling shapes for the tracker and derivation lanes: turn
//! declarations, FC-state reports and views, and navigation guidance in
//! both directions across the script boundary.

use indicate_instrument_feeder::RawStamp;
use indicate_instrument_feeder::fc_state::{FcCommand, FcReport, FcView};
use indicate_instrument_feeder::nav_guidance::{Guidance, NavCounters, NavReject, NavSnapshot};
use indicate_instrument_feeder::turn::TurnDeclaration;
use indicate_instrument_state::{IdentStr, NavData, NavFromTo, NavSource, Stamped};
use serde::{Deserialize, Serialize};

use super::JsStamp;

/// A turn declaration in the writeState dynamics vocabulary.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in super::super) struct JsTurnDeclaration {
    turn_basis: u8,
    turn_rps: f64,
    age_ms: f64,
}

impl From<TurnDeclaration> for JsTurnDeclaration {
    fn from(declaration: TurnDeclaration) -> Self {
        Self {
            turn_basis: declaration.turn_basis,
            turn_rps: declaration.turn_rps,
            age_ms: declaration.age_ms,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsFcCommand {
    arm: bool,
    result: u32,
}

/// One decoded fc-state report as the script hands it over.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in super::super) struct JsFcReport {
    stamp: JsStamp,
    arm_state: u32,
    #[serde(default)]
    last_command: Option<JsFcCommand>,
}

impl TryFrom<JsFcReport> for FcReport {
    type Error = &'static str;

    fn try_from(report: JsFcReport) -> Result<Self, Self::Error> {
        Ok(FcReport {
            stamp: report.stamp.try_into()?,
            arm_state: report.arm_state,
            last_command: report.last_command.map(|command| FcCommand {
                arm: command.arm,
                result: command.result,
            }),
        })
    }
}

/// The fc-state display view.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in super::super) struct JsFcView {
    arm_state: u32,
    last_command: Option<JsFcCommandOut>,
    age_ms: f64,
    stale: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JsFcCommandOut {
    arm: bool,
    result: u32,
}

impl From<FcView> for JsFcView {
    fn from(view: FcView) -> Self {
        Self {
            arm_state: view.arm_state,
            last_command: view.last_command.map(|command| JsFcCommandOut {
                arm: command.arm,
                result: command.result,
            }),
            age_ms: view.age_ms,
            stale: view.stale,
        }
    }
}

/// One decoded guidance sample as the script hands it over. Malformed
/// idents become the INVALID sentinel downstream, never silently
/// truncated text.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in super::super) struct JsGuidanceSample {
    stamp: JsStamp,
    to_ident: String,
    from_ident: String,
    course_rad: f32,
    lateral_deviation_m: f32,
    vertical_deviation_m: f32,
    distance_to_waypoint_m: f32,
    leg_index: u32,
    waypoint_count: u32,
    solution_quality: u32,
}

fn ident_or_invalid(text: &str) -> IdentStr {
    IdentStr::new(text).unwrap_or(IdentStr::INVALID)
}

impl JsGuidanceSample {
    /// The lane input: a stamp (poisoned on shape faults so the tracker
    /// counts an invalid stamp) plus the guidance values.
    pub(in super::super) fn into_lane(self) -> (RawStamp, Guidance) {
        let stamp = RawStamp::try_from(self.stamp).unwrap_or(RawStamp {
            role: 0,
            integrity: 0,
            source_id: 0,
            incarnation: [0; 16],
            epoch: 0,
            sequence: 0,
            acquired_at_ns: 0,
            clock: 0,
        });
        (
            stamp,
            Guidance {
                to_ident: ident_or_invalid(&self.to_ident),
                from_ident: ident_or_invalid(&self.from_ident),
                course_rad: self.course_rad,
                lateral_deviation_m: self.lateral_deviation_m,
                vertical_deviation_m: self.vertical_deviation_m,
                distance_to_waypoint_m: self.distance_to_waypoint_m,
                leg_index: self.leg_index,
                waypoint_count: self.waypoint_count,
                solution_quality: self.solution_quality,
            },
        )
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JsGuidanceOut {
    to_ident: String,
    from_ident: String,
    course_rad: f32,
    lateral_deviation_m: f32,
    vertical_deviation_m: f32,
    distance_to_waypoint_m: f32,
    leg_index: u32,
    waypoint_count: u32,
    solution_quality: u32,
}

/// The guidance snapshot the script consumes.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in super::super) struct JsNavSnapshot {
    nav_guidance: JsGuidanceOut,
    age_ms: f64,
}

impl From<NavSnapshot> for JsNavSnapshot {
    fn from(snapshot: NavSnapshot) -> Self {
        let guidance = snapshot.guidance;
        Self {
            nav_guidance: JsGuidanceOut {
                to_ident: guidance.to_ident.as_str().to_owned(),
                from_ident: guidance.from_ident.as_str().to_owned(),
                course_rad: guidance.course_rad,
                lateral_deviation_m: guidance.lateral_deviation_m,
                vertical_deviation_m: guidance.vertical_deviation_m,
                distance_to_waypoint_m: guidance.distance_to_waypoint_m,
                leg_index: guidance.leg_index,
                waypoint_count: guidance.waypoint_count,
                solution_quality: guidance.solution_quality,
            },
            age_ms: snapshot.age_ms,
        }
    }
}

/// A guidance snapshot arriving FROM script for display conversion.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in super::super) struct JsNavSnapshotIn {
    nav_guidance: JsGuidanceIn,
    age_ms: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JsGuidanceIn {
    #[serde(default)]
    to_ident: String,
    #[serde(default)]
    from_ident: String,
    course_rad: f32,
    lateral_deviation_m: f32,
    vertical_deviation_m: f32,
    distance_to_waypoint_m: f32,
    #[serde(default)]
    leg_index: u32,
    #[serde(default)]
    waypoint_count: u32,
    solution_quality: u32,
}

impl JsNavSnapshotIn {
    pub(in super::super) fn into_snapshot(self) -> NavSnapshot {
        NavSnapshot {
            guidance: Guidance {
                to_ident: ident_or_invalid(&self.nav_guidance.to_ident),
                from_ident: ident_or_invalid(&self.nav_guidance.from_ident),
                course_rad: self.nav_guidance.course_rad,
                lateral_deviation_m: self.nav_guidance.lateral_deviation_m,
                vertical_deviation_m: self.nav_guidance.vertical_deviation_m,
                distance_to_waypoint_m: self.nav_guidance.distance_to_waypoint_m,
                leg_index: self.nav_guidance.leg_index,
                waypoint_count: self.nav_guidance.waypoint_count,
                solution_quality: self.nav_guidance.solution_quality,
            },
            age_ms: self.age_ms,
        }
    }
}

/// The nav-guidance diagnostics with the script's reason strings.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in super::super) struct JsNavDiagnostics {
    accepted: u32,
    invalid_stamps: u32,
    wrong_source: u32,
    duplicates: u32,
    malformed_guidance: u32,
    last_reject_reason: Option<&'static str>,
}

impl JsNavDiagnostics {
    pub(in super::super) fn new(counters: NavCounters, last: Option<NavReject>) -> Self {
        Self {
            accepted: counters.accepted,
            invalid_stamps: counters.invalid_stamps,
            wrong_source: counters.wrong_source,
            duplicates: counters.duplicates,
            malformed_guidance: counters.malformed_guidance,
            last_reject_reason: last.map(|reason| match reason {
                NavReject::InvalidStamp => "invalidStamps",
                NavReject::WrongSource => "wrongSource",
                NavReject::Duplicate => "duplicates",
                NavReject::MalformedGuidance => "malformedGuidance",
            }),
        }
    }
}

/// The nav group in the writeState vocabulary.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in super::super) struct JsNavGroup {
    source: u8,
    fromto: u8,
    course_rad: f32,
    course_reference: u8,
    cdi_dots: f32,
    vdev_dots: Option<f32>,
    dist_nm: Option<f32>,
    to_ident: String,
    from_ident: String,
    age_ms: Option<f32>,
}

impl From<Stamped<NavData>> for JsNavGroup {
    fn from(stamped: Stamped<NavData>) -> Self {
        let nav = stamped.data.unwrap_or_default();
        Self {
            source: match nav.source {
                NavSource::None => 0,
                NavSource::Gps => 1,
                NavSource::Nav1 => 2,
                NavSource::Nav2 => 3,
                NavSource::Unknown => 255,
            },
            fromto: match nav.fromto {
                NavFromTo::Off => 0,
                NavFromTo::To => 1,
                NavFromTo::From => 2,
                NavFromTo::Unknown => 255,
            },
            course_rad: nav.course_rad,
            course_reference: nav.course_reference.to_u8(),
            cdi_dots: nav.cdi_dots,
            vdev_dots: nav.vdev_dots,
            dist_nm: nav.dist_nm,
            to_ident: nav.to_ident.as_str().to_owned(),
            from_ident: nav.from_ident.as_str().to_owned(),
            age_ms: stamped.age_ms,
        }
    }
}
