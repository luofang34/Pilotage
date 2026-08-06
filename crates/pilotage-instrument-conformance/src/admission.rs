//! The admission matrix and its five check families.
//!
//! All geometry tests happen in DESIGN-FRAME space: text runs are
//! reduced to conservative ink rectangles (nominal metrics around the
//! anchor) and mapped through the scene's transform state, exactly as
//! a backend would place them, so a panel cannot move a run out of a
//! check's sight with a `translate`/`rotate` it already uses for
//! legitimate drawing.

use pilotage_instrument_glyphs::PANEL_VOCABULARY;
use pilotage_instrument_registry::{
    CANONICAL_STATES, EMPTY_CONFIG, PanelDescriptor, PanelDrawError, Region, Registry,
};
use pilotage_instrument_scene::{
    Cmd, LayerError, MAX_SCENE_BYTES, SceneCmds, SceneWriter, validate_layers,
};
use pilotage_instrument_state::{
    AircraftState, FreshnessPolicy, GroupId, PanelData, resolve, withhold_group,
};

mod background;
mod geometry;

use background::check_background;
use geometry::{Ctm, Rect, text_rect};

/// One admission run's outcome: how much was covered, and what was
/// tolerated. Failures are typed errors, never entries here.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AdmissionReport {
    /// Drawn and checked panel × state × withholding cases.
    pub cases: usize,
    /// Tolerated-but-counted observations.
    pub warnings: Vec<AdmissionWarning>,
}

/// A tolerated observation, counted so growth is visible.
#[derive(Debug, Clone, PartialEq)]
pub enum AdmissionWarning {
    /// A text run whose ink extends outside the panel's design frame
    /// without a bounding clip.
    FrameOverflow {
        /// The drawing panel.
        panel: &'static str,
        /// The corpus state.
        state: &'static str,
        /// The text run's content.
        text: String,
    },
}

/// Why a panel failed admission.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum AdmissionError {
    /// The panel refused to draw a corpus case.
    #[error("panel {panel} failed to draw state {state} (withheld: {withheld:?})")]
    Draw {
        /// The refusing panel.
        panel: &'static str,
        /// The corpus state.
        state: &'static str,
        /// The withheld group, if the case withholds one.
        withheld: Option<GroupId>,
        /// The panel's own reason.
        #[source]
        source: PanelDrawError,
    },
    /// The emitted scene violates the layer contract.
    #[error("panel {panel} scene for {state} (withheld: {withheld:?}) breaks the layer contract")]
    LayerContract {
        /// The drawing panel.
        panel: &'static str,
        /// The corpus state.
        state: &'static str,
        /// The withheld group, if any.
        withheld: Option<GroupId>,
    },
    /// A required layer band is absent from the emitted scene.
    #[error(
        "panel {panel} scene for {state} (withheld: {withheld:?}) is missing required layers {missing:#04x}"
    )]
    MissingRequiredLayers {
        /// The drawing panel.
        panel: &'static str,
        /// The corpus state.
        state: &'static str,
        /// The withheld group, if any.
        withheld: Option<GroupId>,
        /// Required-but-absent layer bits.
        missing: u8,
    },
    /// The scene does not decode.
    #[error("panel {panel} scene for {state} does not decode")]
    Decode {
        /// The drawing panel.
        panel: &'static str,
        /// The corpus state.
        state: &'static str,
    },
    /// A text run uses a character outside the controlled vocabulary.
    #[error("panel {panel} draws {ch:?} in {state}, outside the controlled vocabulary")]
    GlyphCoverage {
        /// The drawing panel.
        panel: &'static str,
        /// The corpus state.
        state: &'static str,
        /// The uncovered character.
        ch: char,
    },
    /// A withheld group's declared region shows numeric text — the
    /// panel painted numbers for data it was not given.
    #[error(
        "panel {panel} paints numeric text {text:?} in the {group:?} region of {state} with {group:?} withheld"
    )]
    DishonestNumeral {
        /// The drawing panel.
        panel: &'static str,
        /// The corpus state.
        state: &'static str,
        /// The withheld group whose region gained numbers.
        group: GroupId,
        /// The offending run.
        text: String,
    },
    /// The Background band contradicts the declared capability: a
    /// compositor plans around this declaration, so both directions are
    /// refused — painting a band declared `NotUsed`, and failing to
    /// opaquely cover a band declared owned.
    #[error("panel {panel} declares background {declared} but its {state} scene {defect} the band")]
    BackgroundContract {
        /// The drawing panel.
        panel: &'static str,
        /// The corpus state.
        state: &'static str,
        /// The declared capability.
        declared: &'static str,
        /// What the scene actually did: "paints" or "does not cover".
        defect: &'static str,
    },
    /// The nothing-fed furniture itself paints numerals inside a
    /// declared region — an always-fabricating panel would otherwise
    /// launder its numbers into the honest-status baseline.
    #[error(
        "panel {panel} paints numeric furniture {text:?} inside its {group:?} region with nothing fed"
    )]
    DishonestFurniture {
        /// The drawing panel.
        panel: &'static str,
        /// The group whose region carries the furniture numerals.
        group: GroupId,
        /// The offending run.
        text: String,
    },
}

/// One decoded text run as a conservative design-space ink rectangle.
#[derive(Debug, Clone, PartialEq)]
struct TextRun {
    rect: Rect,
    text: String,
    /// Whether a clip was active when the run painted.
    clipped: bool,
    /// Whether the ink rectangle intersects the active clip — a tape
    /// label scrolled past its strip's clip edge paints nothing.
    visible: bool,
}

impl TextRun {
    fn numeric(&self) -> bool {
        self.text.chars().any(|c| c.is_ascii_digit())
    }

    fn in_region(&self, region: &Region) -> bool {
        self.rect.intersects(&Rect {
            min_x: region.x,
            min_y: region.y,
            max_x: region.x + region.width,
            max_y: region.y + region.height,
        })
    }
}

/// Runs the full admission matrix over `registry`.
pub fn admit(registry: &Registry) -> Result<AdmissionReport, AdmissionError> {
    let mut report = AdmissionReport::default();
    for panel in registry.panels() {
        admit_panel(panel, &mut report)?;
    }
    Ok(report)
}

fn admit_panel(
    panel: &'static PanelDescriptor,
    report: &mut AdmissionReport,
) -> Result<(), AdmissionError> {
    // The nothing-fed furniture must be numeral-free inside every
    // declared region: honest degradation shows dashes and labels
    // there, and an always-fabricating panel must not be able to
    // launder its numbers into the baseline.
    let nothing_fed = &CANONICAL_STATES[0];
    let furniture = draw_runs(panel, nothing_fed.id, None, (nothing_fed.build)())?;
    for (group, region) in panel.group_regions {
        for run in &furniture {
            if run.numeric() && run.visible && run.in_region(region) {
                return Err(AdmissionError::DishonestFurniture {
                    panel: panel.id,
                    group: *group,
                    text: run.text.clone(),
                });
            }
        }
    }
    let states = CANONICAL_STATES
        .iter()
        .map(|s| (s.id, s.build))
        .chain(panel.extreme_states.iter().map(|e| (e.id, e.build)));
    for (state_id, build) in states {
        check_case(panel, state_id, None, build(), report)?;
        for group in GroupId::ALL {
            if panel.required_groups.contains(group) {
                let withheld = withhold_group(&build(), group);
                check_case(panel, state_id, Some(group), withheld, report)?;
            }
        }
    }
    Ok(())
}

fn check_case(
    panel: &'static PanelDescriptor,
    state_id: &'static str,
    withheld: Option<GroupId>,
    state: AircraftState,
    report: &mut AdmissionReport,
) -> Result<(), AdmissionError> {
    let runs = draw_runs(panel, state_id, withheld, state)?;
    let frame = Rect {
        min_x: 0.0,
        min_y: 0.0,
        max_x: panel.design_frame.width,
        max_y: panel.design_frame.height,
    };
    for run in &runs {
        for ch in run.text.chars() {
            if !PANEL_VOCABULARY.contains(&ch) {
                return Err(AdmissionError::GlyphCoverage {
                    panel: panel.id,
                    state: state_id,
                    ch,
                });
            }
        }
        if !frame.contains(&run.rect) && !run.clipped {
            report.warnings.push(AdmissionWarning::FrameOverflow {
                panel: panel.id,
                state: state_id,
                text: run.text.clone(),
            });
        }
    }
    if let Some(group) = withheld {
        check_honest_status(panel, state_id, group, &runs)?;
    }
    report.cases += 1;
    Ok(())
}

fn check_honest_status(
    panel: &'static PanelDescriptor,
    state_id: &'static str,
    group: GroupId,
    runs: &[TextRun],
) -> Result<(), AdmissionError> {
    for (region_group, region) in panel.group_regions {
        if *region_group != group {
            continue;
        }
        for run in runs {
            if run.numeric() && run.visible && run.in_region(region) {
                return Err(AdmissionError::DishonestNumeral {
                    panel: panel.id,
                    state: state_id,
                    group,
                    text: run.text.clone(),
                });
            }
        }
    }
    Ok(())
}

fn draw_runs(
    panel: &'static PanelDescriptor,
    state_id: &'static str,
    withheld: Option<GroupId>,
    state: AircraftState,
) -> Result<Vec<TextRun>, AdmissionError> {
    let data = resolve(&state, &FreshnessPolicy::default());
    let mut buf = vec![0u8; MAX_SCENE_BYTES];
    let scene = draw_scene(panel, &data, &mut buf).map_err(|source| AdmissionError::Draw {
        panel: panel.id,
        state: state_id,
        withheld,
        source,
    })?;
    let layers = validate_layers(scene).map_err(|error| match error {
        LayerError::Decode(_) => AdmissionError::Decode {
            panel: panel.id,
            state: state_id,
        },
        _ => AdmissionError::LayerContract {
            panel: panel.id,
            state: state_id,
            withheld,
        },
    })?;
    let missing = panel.required_layers & !layers.present;
    if missing != 0 {
        return Err(AdmissionError::MissingRequiredLayers {
            panel: panel.id,
            state: state_id,
            withheld,
            missing,
        });
    }
    check_background(panel, state_id, scene)?;
    collect_runs(scene).ok_or(AdmissionError::Decode {
        panel: panel.id,
        state: state_id,
    })
}

fn draw_scene<'b>(
    panel: &PanelDescriptor,
    data: &PanelData,
    buf: &'b mut [u8],
) -> Result<&'b [u8], PanelDrawError> {
    let mut writer = SceneWriter::new(buf)?;
    (panel.draw)(data, &EMPTY_CONFIG, None, &mut writer)?;
    let used = writer.finish();
    Ok(buf.get(..used).unwrap_or(&[]))
}

/// Decodes every text run into a design-space ink rectangle, tracking
/// the transform and clip state the way a backend would.
fn collect_runs(scene: &[u8]) -> Option<Vec<TextRun>> {
    let cmds = SceneCmds::new(scene).ok()?;
    let mut runs = Vec::new();
    let mut stack = vec![(Ctm::IDENTITY, None::<Rect>)];
    for cmd in cmds {
        match cmd {
            Ok(Cmd::Text {
                x,
                y,
                size,
                anchor,
                text,
            }) => {
                let (ctm, clip) = stack.last().copied()?;
                let local = text_rect(x, y, size, anchor.h, anchor.v, text.chars().count());
                let rect = ctm.map_rect(&local);
                runs.push(TextRun {
                    visible: clip.is_none_or(|clip| rect.intersects(&clip)),
                    rect,
                    text: text.to_string(),
                    clipped: clip.is_some(),
                });
            }
            Ok(Cmd::Save) => stack.push(stack.last().copied()?),
            Ok(Cmd::Restore) => {
                stack.pop();
                if stack.is_empty() {
                    stack.push((Ctm::IDENTITY, None));
                }
            }
            Ok(Cmd::Translate { x, y }) => {
                if let Some((ctm, _)) = stack.last_mut() {
                    ctm.translate(x, y);
                }
            }
            Ok(Cmd::Rotate { radians }) => {
                if let Some((ctm, _)) = stack.last_mut() {
                    ctm.rotate(radians);
                }
            }
            Ok(Cmd::ClipRect { x, y, w, h }) => {
                if let Some((ctm, clip)) = stack.last_mut() {
                    let mapped = ctm.map_rect(&Rect {
                        min_x: x,
                        min_y: y,
                        max_x: x + w,
                        max_y: y + h,
                    });
                    *clip = Some(match clip {
                        None => mapped,
                        Some(previous) => previous.intersect(&mapped),
                    });
                }
            }
            Ok(_) => {}
            Err(_) => return None,
        }
    }
    Some(runs)
}

#[cfg(test)]
mod tests;
