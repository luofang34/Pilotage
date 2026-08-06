//! The background-contract family: the declared capability must be the
//! Background band's actual behavior in every corpus case.

use pilotage_instrument_registry::{BackgroundCapability, PanelDescriptor};
use pilotage_instrument_scene::{Cmd, LayerId, PaintMode, SceneCmds};

use super::AdmissionError;
use super::geometry::{Ctm, Rect};

/// The declared background capability must be the scene's actual
/// behavior in every corpus case: `NotUsed` may not paint in the band
/// (opening and closing it empty is tolerated); `Opaque` and
/// `Cedeable` must own it with a full-frame opaque paint. Coverage is
/// proven by an axis-aligned, unclipped (or frame-covering-clip),
/// full-alpha `Rect` fill — the shipped ground pattern; a panel that
/// builds its ground purely from polygons lays a base rect first, and
/// the refusal message names this rule. `Cedeable`'s ceding under
/// configuration is pinned by the panel's own byte-equivalence tests —
/// the harness draws the empty config, so it verifies the band-owning
/// default.
pub(super) fn check_background(
    panel: &'static PanelDescriptor,
    state_id: &'static str,
    scene: &[u8],
) -> Result<(), AdmissionError> {
    let (painted, covered) =
        scan_background(scene, panel.design_frame.width, panel.design_frame.height).ok_or(
            AdmissionError::Decode {
                panel: panel.id,
                state: state_id,
            },
        )?;
    let (declared, defect) = match panel.background {
        BackgroundCapability::NotUsed if painted => ("NotUsed", "paints"),
        BackgroundCapability::Opaque if !covered => (
            "Opaque",
            "does not opaquely cover (an axis-aligned unclipped full-frame rect fill)",
        ),
        BackgroundCapability::Cedeable if !covered => (
            "Cedeable",
            "does not opaquely cover (an axis-aligned unclipped full-frame rect fill)",
        ),
        _ => return Ok(()),
    };
    Err(AdmissionError::BackgroundContract {
        panel: panel.id,
        state: state_id,
        declared,
        defect,
    })
}

/// Graphics state as the real state machine carries it: Save pushes
/// transform, clip, and paint state together; Restore pops all three.
#[derive(Clone, Copy)]
struct Gs {
    ctm: Ctm,
    clip: Option<Rect>,
    fill_alpha: u8,
}

impl Gs {
    const DEFAULT: Self = Self {
        ctm: Ctm::IDENTITY,
        clip: None,
        fill_alpha: 255,
    };
}

/// Whether any paint lands in the Background band, and whether the
/// band carries a proven full-frame opaque fill: an axis-aligned
/// full-alpha `Rect` whose mapped bounds contain the frame and whose
/// active clip (if any) also contains the frame.
fn scan_background(scene: &[u8], width: f32, height: f32) -> Option<(bool, bool)> {
    let cmds = SceneCmds::new(scene).ok()?;
    let frame = Rect {
        min_x: 0.0,
        min_y: 0.0,
        max_x: width,
        max_y: height,
    };
    let mut in_background = false;
    let mut painted = false;
    let mut covered = false;
    let mut stack = vec![Gs::DEFAULT];
    for cmd in cmds {
        let cmd = cmd.ok()?;
        match cmd {
            Cmd::BeginLayer {
                layer: LayerId::Background,
            } => in_background = true,
            Cmd::EndLayer {
                layer: LayerId::Background,
            } => in_background = false,
            _ => {}
        }
        track_state(&mut stack, &cmd);
        if in_background && paints(&cmd) {
            painted = true;
            if covers_frame(stack.last()?, &cmd, &frame) {
                covered = true;
            }
        }
    }
    Some((painted, covered))
}

/// Applies a state command to the graphics-state stack, exactly as the
/// real state machine would.
fn track_state(stack: &mut Vec<Gs>, cmd: &Cmd<'_>) {
    match *cmd {
        Cmd::Save => {
            if let Some(top) = stack.last().copied() {
                stack.push(top);
            }
        }
        Cmd::Restore => {
            stack.pop();
            if stack.is_empty() {
                stack.push(Gs::DEFAULT);
            }
        }
        Cmd::Translate { x, y } => {
            if let Some(gs) = stack.last_mut() {
                gs.ctm.translate(x, y);
            }
        }
        Cmd::Rotate { radians } => {
            if let Some(gs) = stack.last_mut() {
                gs.ctm.rotate(radians);
            }
        }
        Cmd::FillColor { color } => {
            if let Some(gs) = stack.last_mut() {
                gs.fill_alpha = color.a;
            }
        }
        Cmd::ClipRect { x, y, w, h } => {
            if let Some(gs) = stack.last_mut() {
                let mapped = gs.ctm.map_rect(&Rect {
                    min_x: x,
                    min_y: y,
                    max_x: x + w,
                    max_y: y + h,
                });
                gs.clip = Some(match gs.clip {
                    None => mapped,
                    Some(previous) => previous.intersect(&mapped),
                });
            }
        }
        _ => {}
    }
}

/// Whether this command is a proven full-frame opaque fill under the
/// active graphics state. Exact, not conservative: only an axis-aligned
/// map makes bbox containment equal actual coverage, and the active
/// clip must itself contain the frame or the fill is cropped below
/// full coverage.
fn covers_frame(gs: &Gs, cmd: &Cmd<'_>, frame: &Rect) -> bool {
    let Cmd::Rect { mode, x, y, w, h } = *cmd else {
        return false;
    };
    let clip_ok = gs.clip.is_none_or(|clip| clip.contains(frame));
    if gs.fill_alpha != 255
        || !matches!(mode, PaintMode::Fill | PaintMode::FillStroke)
        || !gs.ctm.is_axis_aligned()
        || !clip_ok
    {
        return false;
    }
    let bbox = gs.ctm.map_rect(&Rect {
        min_x: x,
        min_y: y,
        max_x: x + w,
        max_y: y + h,
    });
    bbox.contains(frame)
}

/// Whether a command puts ink on the surface (state and structure
/// commands do not).
fn paints(cmd: &Cmd<'_>) -> bool {
    matches!(
        cmd,
        Cmd::Rect { .. }
            | Cmd::Circle { .. }
            | Cmd::Arc { .. }
            | Cmd::Line { .. }
            | Cmd::Text { .. }
            | Cmd::Polyline { .. }
            | Cmd::Polygon { .. }
    )
}
