//! The mission fleet file: the survey objective, the lead over the whole
//! mission, one section per landmark with its aircraft, and an optional
//! camera fault the run must recover from.

use std::path::Path;

use pilotage_client_session::mission::SurveyFile;

use crate::error::XtaskError;

/// A latitude, longitude and altitude, in degrees and metres.
pub(crate) type Launch = [f64; 3];

/// The whole fleet file.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MissionFile {
    /// The survey every run of this fleet starts from.
    pub(crate) objective: SurveyFile,
    /// The lead over every section.
    pub(crate) command: Lead,
    /// The sections, in launch order.
    pub(crate) sections: Vec<Section>,
    /// The energy model every aircraft plans with.
    pub(crate) airframe: Airframe,
    /// A camera fault on one aircraft's planned line.
    #[serde(default)]
    pub(crate) fault: Option<Fault>,
}

/// A coordinator with no aircraft of its own.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Lead {
    /// The lane name.
    pub(crate) name: String,
    /// Where its members' plans start from.
    pub(crate) launch: Launch,
}

/// One section lead and its aircraft.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Section {
    /// The lane name.
    pub(crate) name: String,
    /// The landmark the section is named for, for people.
    pub(crate) landmark: String,
    /// Where the section's share of the mission starts from.
    pub(crate) launch: Launch,
    /// The section's aircraft.
    pub(crate) aircraft: Vec<Aircraft>,
}

/// One simulated aircraft.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Aircraft {
    /// The short id; the lane is `SIM-<id>`.
    pub(crate) id: String,
    /// Where it takes off.
    pub(crate) launch: Launch,
}

/// The energy every aircraft plans with.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Airframe {
    /// Usable battery energy, Wh.
    pub(crate) energy_wh: f64,
    /// Power drawn in cruise, W.
    pub(crate) cruise_power_w: f64,
}

/// A camera fault along one planned line.
#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Fault {
    /// The aircraft id whose camera fails.
    pub(crate) aircraft: String,
    /// The task id of the line, in that aircraft's own plan.
    pub(crate) line: String,
    /// The camera fault kind: `dark`, `blur` or `overexposed`.
    pub(crate) kind: String,
}

impl Aircraft {
    /// The aircraft's lane name.
    pub(crate) fn lane(&self) -> String {
        format!("SIM-{}", self.id)
    }
}

impl MissionFile {
    /// Reads and checks the fleet file at `path`.
    ///
    /// # Errors
    ///
    /// Returns [`XtaskError::MissionFile`] when the file cannot be read or
    /// parsed, names no section, or repeats a lane name.
    pub(crate) fn load_blocking(path: &Path) -> Result<Self, XtaskError> {
        let fail = |reason: String| XtaskError::MissionFile {
            path: path.to_path_buf(),
            reason,
        };
        let text = std::fs::read_to_string(path).map_err(|e| fail(e.to_string()))?;
        let file: Self = serde_json::from_str(&text).map_err(|e| fail(e.to_string()))?;
        file.check().map_err(fail)?;
        Ok(file)
    }

    fn check(&self) -> Result<(), String> {
        if self.sections.iter().all(|s| s.aircraft.is_empty()) {
            return Err("the file lists no aircraft".to_owned());
        }
        let mut names = vec![self.command.name.clone()];
        for section in &self.sections {
            if section.aircraft.is_empty() {
                return Err(format!("section {} has no aircraft", section.name));
            }
            names.push(section.name.clone());
            names.extend(section.aircraft.iter().map(Aircraft::lane));
        }
        let mut sorted = names.clone();
        sorted.sort();
        if let Some(pair) = sorted.windows(2).find(|pair| pair[0] == pair[1]) {
            return Err(format!("the lane name {} is used twice", pair[0]));
        }
        if let Some(fault) = &self.fault
            && !self.aircraft().any(|a| a.id == fault.aircraft)
        {
            return Err(format!("the fault names no aircraft {}", fault.aircraft));
        }
        Ok(())
    }

    /// Every aircraft, in launch order.
    pub(crate) fn aircraft(&self) -> impl Iterator<Item = &Aircraft> {
        self.sections.iter().flat_map(|s| s.aircraft.iter())
    }
}
