//! The admission matrix and its five check families.
//!
//! All geometry tests happen in DESIGN-FRAME space: text runs are
//! reduced to conservative ink rectangles (nominal metrics around the
//! anchor) and mapped through the scene's transform state, exactly as
//! a backend would place them, so a panel cannot move a run out of a
//! check's sight with a `translate`/`rotate` it already uses for
//! legitimate drawing.
//!
//! Honest status is a provenance rule, not a positional one: every
//! numeric run must claim the state group its value derives from
//! ([`Cmd::Attribute`]), and a claimed run may not be visible when its
//! group shows no value — wherever it is drawn. Declared
//! `group_regions` no longer drive this family; they remain the
//! descriptor's statement of which readout surface belongs to which
//! group (the dash-out declaration a shell may present).

use pilotage_instrument_glyphs::PANEL_VOCABULARY;
use pilotage_instrument_registry::{
    CANONICAL_STATES, EMPTY_CONFIG, PanelDescriptor, PanelDrawError, Registry,
};
use pilotage_instrument_scene::{
    Cmd, LayerError, MAX_SCENE_BYTES, SceneCmds, SceneWriter, validate_layers,
};
use pilotage_instrument_state::{
    AircraftState, FreshnessPolicy, GroupId, PanelData, resolve, withhold_group,
};

mod background;
mod geometry;
mod provenance;

use background::check_background;
use geometry::{Ctm, Rect, text_rect};
use provenance::check_provenance;

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
    /// A visible run claims a group that shows no value in the drawn
    /// state — the panel painted a number for data it was not given.
    #[error(
        "panel {panel} shows {text:?} claimed from {group:?} in {state} while {group:?} shows no value"
    )]
    FabricatedNumeral {
        /// The drawing panel.
        panel: &'static str,
        /// The corpus state.
        state: &'static str,
        /// The claimed group.
        group: GroupId,
        /// The offending run.
        text: String,
    },
    /// A numeric run carries no provenance claim. Totality is what
    /// makes the claim rule sound: an unclaimed numeral would escape
    /// every withholding case.
    #[error("panel {panel} draws numeric text {text:?} in {state} with no provenance claim")]
    UntaggedNumeral {
        /// The drawing panel.
        panel: &'static str,
        /// The corpus state.
        state: &'static str,
        /// The unclaimed run.
        text: String,
    },
    /// A run claims a group outside the panel's required set (or an
    /// unknown tag) — a claim the withholding matrix could never test.
    #[error(
        "panel {panel} claims tag {tag:#04x} for {text:?} in {state}, outside its required groups"
    )]
    ForeignClaim {
        /// The drawing panel.
        panel: &'static str,
        /// The corpus state.
        state: &'static str,
        /// The claimed tag byte.
        tag: u8,
        /// The claiming run.
        text: String,
    },
    /// A visible run claims configuration provenance under the
    /// harness's fixed empty configuration — it derives from nothing.
    #[error(
        "panel {panel} shows {text:?} in {state} claiming configuration provenance under the empty configuration"
    )]
    ConfigClaim {
        /// The drawing panel.
        panel: &'static str,
        /// The corpus state.
        state: &'static str,
        /// The claiming run.
        text: String,
    },
    /// A provenance claim not immediately followed by the text run it
    /// covers — a dangling or stacked claim is structurally malformed.
    #[error("panel {panel} scene for {state} carries a provenance claim that covers no text run")]
    MisplacedClaim {
        /// The drawing panel.
        panel: &'static str,
        /// The corpus state.
        state: &'static str,
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
}

/// One decoded text run as a conservative design-space ink rectangle.
#[derive(Debug, Clone, PartialEq)]
struct TextRun {
    rect: Rect,
    text: String,
    /// The provenance claim prefixing the run, if any.
    attribution: Option<u8>,
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
    let data = resolve(&state, &FreshnessPolicy::default());
    let runs = draw_runs(panel, state_id, withheld, &data)?;
    check_provenance(panel, state_id, withheld, &runs)?;
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
    report.cases += 1;
    Ok(())
}

fn draw_runs(
    panel: &'static PanelDescriptor,
    state_id: &'static str,
    withheld: Option<GroupId>,
    data: &PanelData,
) -> Result<Vec<TextRun>, AdmissionError> {
    let mut buf = vec![0u8; MAX_SCENE_BYTES];
    let scene = draw_scene(panel, data, &mut buf).map_err(|source| AdmissionError::Draw {
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
    match collect_runs(scene) {
        Ok(runs) => Ok(runs),
        Err(RunsDefect::Decode) => Err(AdmissionError::Decode {
            panel: panel.id,
            state: state_id,
        }),
        Err(RunsDefect::MisplacedClaim) => Err(AdmissionError::MisplacedClaim {
            panel: panel.id,
            state: state_id,
        }),
    }
}

/// Why the run scanner refused a scene.
enum RunsDefect {
    Decode,
    MisplacedClaim,
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
/// the transform and clip state the way a backend would, and pairing
/// each run with the provenance claim prefixing it. A claim not
/// immediately consumed by a text run is refused — stacked, dangling,
/// or shape-interposed claims are structurally malformed.
fn collect_runs(scene: &[u8]) -> Result<Vec<TextRun>, RunsDefect> {
    let cmds = SceneCmds::new(scene).map_err(|_| RunsDefect::Decode)?;
    let mut runs = Vec::new();
    let mut pending: Option<u8> = None;
    let mut stack = vec![(Ctm::IDENTITY, None::<Rect>)];
    for cmd in cmds {
        if pending.is_some() && !matches!(cmd, Ok(Cmd::Text { .. })) {
            return Err(RunsDefect::MisplacedClaim);
        }
        match cmd {
            Ok(Cmd::Attribute { group }) => pending = Some(group),
            Ok(Cmd::Text {
                x,
                y,
                size,
                anchor,
                text,
            }) => {
                let (ctm, clip) = stack.last().copied().ok_or(RunsDefect::Decode)?;
                let local = text_rect(x, y, size, anchor.h, anchor.v, text.chars().count());
                let rect = ctm.map_rect(&local);
                runs.push(TextRun {
                    visible: clip.is_none_or(|clip| rect.intersects(&clip)),
                    rect,
                    text: text.to_string(),
                    attribution: pending.take(),
                    clipped: clip.is_some(),
                });
            }
            Ok(Cmd::Save) => stack.push(stack.last().copied().ok_or(RunsDefect::Decode)?),
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
            Err(_) => return Err(RunsDefect::Decode),
        }
    }
    if pending.is_some() {
        return Err(RunsDefect::MisplacedClaim);
    }
    Ok(runs)
}

#[cfg(test)]
mod tests;
