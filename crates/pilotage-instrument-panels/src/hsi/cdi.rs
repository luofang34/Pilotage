//! The course deviation indicator: course arrow, deviation bar, scale
//! dots, and TO/FROM triangle.

use pilotage_instrument_scene::{PaintMode, Rgba8, SceneError, SceneWriter};
use pilotage_instrument_state::{NavFromTo, NavResolved, NavSource};

use crate::palette;

/// Two dots of lateral deviation = 75 px (37.5 px/dot, G5 proportions).
const PX_PER_DOT: f32 = 37.5;

pub(crate) fn source_color(source: NavSource) -> Rgba8 {
    match source {
        NavSource::Gps => palette::MAGENTA,
        _ => palette::GREEN,
    }
}

/// Draws the CDI in the rose frame, rotated to the selected course.
pub fn draw_cdi(
    scene: &mut SceneWriter<'_>,
    nav: &NavResolved,
    heading_rad: f32,
) -> Result<(), SceneError> {
    let color = source_color(nav.data.source);
    scene.save()?;
    scene.translate(super::CX, super::CY)?;
    scene.rotate(nav.data.course_rad - heading_rad)?;

    // Course arrow: head, fore shaft, aft shaft.
    scene.fill_color(color)?;
    scene.stroke(color, 4.0)?;
    scene.polygon(
        PaintMode::Fill,
        &[[0.0, -90.0], [-10.0, -70.0], [10.0, -70.0]],
    )?;
    scene.line(0.0, -70.0, 0.0, -38.0)?;
    scene.line(0.0, 38.0, 0.0, 90.0)?;

    // Scale dots on the perpendicular.
    scene.stroke(palette::GREY, 2.0)?;
    for dx in [-2.0f32, -1.0, 1.0, 2.0] {
        scene.circle(PaintMode::Stroke, dx * PX_PER_DOT, 0.0, 4.0)?;
    }

    // Deviation bar: where the course line *is*, relative to the aircraft.
    let dx = (nav.data.cdi_dots.clamp(-2.4, 2.4)) * PX_PER_DOT;
    scene.fill_color(color)?;
    scene.rect(PaintMode::Fill, dx - 2.5, -36.0, 5.0, 72.0)?;

    // TO/FROM triangle beside the fore shaft.
    match nav.data.fromto {
        NavFromTo::To => {
            scene.polygon(
                PaintMode::Fill,
                &[[0.0, -34.0], [-8.0, -18.0], [8.0, -18.0]],
            )?;
        }
        NavFromTo::From => {
            scene.polygon(PaintMode::Fill, &[[0.0, 34.0], [-8.0, 18.0], [8.0, 18.0]])?;
        }
        NavFromTo::Off => {}
    }
    scene.restore()?;
    Ok(())
}
