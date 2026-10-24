//! What a revised survey objective changes. The area, the return to the
//! launch point and the evidence each line needs are what the survey must
//! achieve. The height is how it flies, unless the objective holds the
//! height: then the operator requires it. Speed, sidelap, field of view,
//! line bearing and launch are how it flies. The title, the identity and
//! the revision number are no content, so an objective with the same
//! identity and other content still changes.

use pilotage_protocol::wire;

use crate::change::{ChangeClass, ChangeSet};

/// The changes from objective `before` to `after`.
#[must_use]
pub fn classify(before: &wire::MissionObjective, after: &wire::MissionObjective) -> ChangeSet {
    let mut changes = ChangeSet::default();
    let survey = |o: &wire::MissionObjective| {
        o.kind
            .as_ref()
            .map(|wire::mission_objective::Kind::Survey(s)| s.clone())
    };
    match (survey(before), survey(after)) {
        (Some(before), Some(after)) => surveys(&before, &after, &mut changes),
        (None, None) => {}
        _ => changes.add(ChangeClass::Objective, "the kind of objective"),
    }
    changes
}

fn surveys(before: &wire::SurveyObjective, after: &wire::SurveyObjective, set: &mut ChangeSet) {
    use ChangeClass::{Objective, Plan};
    if before.area != after.area {
        set.add(Objective, "the area");
    }
    if before.return_to_launch != after.return_to_launch {
        set.add(Objective, "the return to the launch point");
    }
    let need = |s: &wire::SurveyObjective| s.min_usable_frames_per_line.max(1);
    if need(before) != need(after) {
        set.add(Objective, "the evidence each line needs");
    }
    let height_changed = before.height_m.to_bits() != after.height_m.to_bits();
    if before.hold_height || after.hold_height {
        if height_changed || before.hold_height != after.hold_height {
            set.add(Objective, "the height the operator holds");
        }
    } else if height_changed {
        set.add(Plan, "the height");
    }
    let differs = |a: f64, b: f64| a.to_bits() != b.to_bits();
    if differs(before.cruise_mps, after.cruise_mps) {
        set.add(Plan, "the speed");
    }
    if differs(before.sidelap, after.sidelap)
        || differs(before.across_track_fov_deg, after.across_track_fov_deg)
    {
        set.add(Plan, "the line spacing");
    }
    if before.line_bearing_deg.map(f64::to_bits) != after.line_bearing_deg.map(f64::to_bits) {
        set.add(Plan, "the line bearing");
    }
    if before.launch != after.launch || differs(before.launch_altitude_m, after.launch_altitude_m) {
        set.add(Plan, "the launch point");
    }
}
