//! The themable half of the palette, as validated data (ADR-0029).
//!
//! A shell supplies a [`Theme`]; only a [`ValidTheme`] can reach drawing
//! code. The never-skinnable half does not depend on that boundary:
//! safety colors are separate constants in [`crate::safety`], and a
//! structure gate keeps their palette aliases inside this crate, whether
//! or not a shell supplies a theme. Validation enforces what "themable"
//! is allowed to mean:
//!
//! - **Imitation**: no themable color may sit within
//!   [`SAFETY_DISTANCE_MIN`] (max-channel RGB distance) of any warning
//!   hue — the four colored safety constants and the whole
//!   red-through-yellow segment between them — so a theme cannot
//!   manufacture or camouflage failure and caution colors.
//!   [`crate::safety::ANNUNCIATION_WHITE`] is exempt by design: primary
//!   symbology is legitimately white, and advisory rows signal by stack
//!   position, not hue.
//! - **Contrast**: primary symbology must keep at least
//!   [`CONTRAST_LUMA_MIN`] of luma against every ground it draws over —
//!   the panel, the readout boxes, both halves of the horizon, and the
//!   tape background composited over each horizon half at its declared
//!   alpha. Separately, the colored annunciation hues must keep
//!   [`SAFETY_CONTRAST_LUMA_MIN`] of luma against the panel and box
//!   grounds they paint over, so a theme cannot bury a failure flag in
//!   an equiluminant ground (the red-on-green deficiency case).
//! - **Opacity**: every themable color paints at full alpha except the
//!   tape background, which keeps at least [`TAPE_ALPHA_MIN`] so tapes
//!   stay legible over any horizon — no theme can fade symbology
//!   toward invisibility.
//! - **Source identity**: GPS and radio-nav guidance colors must stay
//!   [`SAFETY_DISTANCE_MIN`] apart — source class survives theming.
//!
//! The thresholds are an explicit assurance decision, not an emergent
//! property of the defaults.

use pilotage_instrument_scene::Rgba8;

use crate::safety;

/// Minimum max-channel RGB distance from every safety color, and
/// between the two guidance-source colors.
pub const SAFETY_DISTANCE_MIN: u8 = 64;

/// Minimum luma difference between primary symbology and each ground.
pub const CONTRAST_LUMA_MIN: u8 = 96;

/// Minimum luma difference between each colored annunciation hue and
/// the panel/box grounds it paints over. Sits below
/// [`CONTRAST_LUMA_MIN`] deliberately: failure red holds only 76 luma
/// against the shipped black, so this floor defends against a theme
/// burying safety paint in an equiluminant ground, not against ordinary
/// low contrast.
pub const SAFETY_CONTRAST_LUMA_MIN: u8 = 64;

/// Minimum alpha of the semi-transparent tape background.
pub const TAPE_ALPHA_MIN: u8 = 96;

/// The shell-suppliable colors. Everything failure/caution/alert-class
/// shaped is deliberately absent — see [`crate::safety`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Sky half of the attitude ball.
    pub sky: Rgba8,
    /// Ground half of the attitude ball.
    pub ground: Rgba8,
    /// Primary symbology and scale marks.
    pub primary: Rgba8,
    /// Panel background.
    pub panel_bg: Rgba8,
    /// Semi-transparent tape/box background over the horizon.
    pub tape_bg: Rgba8,
    /// Solid readout-box background.
    pub box_bg: Rgba8,
    /// Box outlines and secondary marks.
    pub outline: Rgba8,
    /// GPS guidance, trends, and rate cues.
    pub gps: Rgba8,
    /// Radio-nav (VLOC) guidance.
    pub radio_nav: Rgba8,
    /// Pilot selections: bugs, selected values, baro.
    pub selection: Rgba8,
    /// Normal-range band on the speed tape.
    pub band_normal: Rgba8,
}

impl Theme {
    /// The shipped look — exactly the [`crate::palette`] constants, so
    /// adopting the theme path repaints nothing.
    pub const DEFAULT: Theme = Theme {
        sky: Rgba8::rgb(0, 110, 210),
        ground: Rgba8::rgb(140, 96, 44),
        primary: Rgba8::rgb(255, 255, 255),
        panel_bg: Rgba8::rgb(0, 0, 0),
        tape_bg: Rgba8::rgba(20, 20, 20, 150),
        box_bg: Rgba8::rgb(0, 0, 0),
        outline: Rgba8::rgb(128, 128, 128),
        gps: Rgba8::rgb(255, 0, 255),
        radio_nav: Rgba8::rgb(0, 255, 0),
        selection: Rgba8::rgb(0, 255, 255),
        band_normal: Rgba8::rgb(0, 160, 0),
    };

    const fn all(&self) -> [(&'static str, Rgba8); 11] {
        // Exhaustive destructuring: a new field fails to compile until
        // it joins the validated list.
        let Theme {
            sky,
            ground,
            primary,
            panel_bg,
            tape_bg,
            box_bg,
            outline,
            gps,
            radio_nav,
            selection,
            band_normal,
        } = *self;
        [
            ("sky", sky),
            ("ground", ground),
            ("primary", primary),
            ("panel_bg", panel_bg),
            ("tape_bg", tape_bg),
            ("box_bg", box_bg),
            ("outline", outline),
            ("gps", gps),
            ("radio_nav", radio_nav),
            ("selection", selection),
            ("band_normal", band_normal),
        ]
    }
}

/// Why a [`Theme`] was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ThemeError {
    /// A themable color sits close enough to a safety hue to imitate
    /// or camouflage it.
    #[error("theme color {field} imitates the {near} safety hue")]
    ImitatesSafetyColor {
        /// The offending theme field.
        field: &'static str,
        /// The safety hue it approaches.
        near: &'static str,
    },
    /// Primary symbology lacks contrast against a ground it draws over.
    #[error("primary symbology holds {delta} luma against {ground}, floor {CONTRAST_LUMA_MIN}")]
    InsufficientContrast {
        /// The ground the primary color fails against.
        ground: &'static str,
        /// The measured luma difference.
        delta: u8,
    },
    /// A color is more transparent than its floor allows.
    #[error("{field} alpha {alpha} below its floor {floor}")]
    AlphaBelowFloor {
        /// The offending theme field.
        field: &'static str,
        /// The measured alpha.
        alpha: u8,
        /// The floor it missed.
        floor: u8,
    },
    /// A colored annunciation hue would be buried against a ground it
    /// paints over.
    #[error(
        "safety hue {hue} holds {delta} luma against {ground}, floor {SAFETY_CONTRAST_LUMA_MIN}"
    )]
    SafetyContrastBuried {
        /// The safety hue at risk.
        hue: &'static str,
        /// The themable ground that would bury it.
        ground: &'static str,
        /// The measured luma difference.
        delta: u8,
    },
    /// GPS and radio-nav guidance colors are not distinguishable.
    #[error("gps and radio_nav colors are not distinguishable")]
    IndistinctSourceColors,
}

/// The largest per-channel RGB difference (alpha excluded).
fn max_channel_distance(a: Rgba8, b: Rgba8) -> u8 {
    let dr = a.r.abs_diff(b.r);
    let dg = a.g.abs_diff(b.g);
    let db = a.b.abs_diff(b.b);
    dr.max(dg).max(db)
}

/// Distance from the red-through-yellow hue segment `(255, g, 0)` for
/// `g` in `0..=255`, which passes through every warning hue between
/// [`safety::FAILURE_RED`] and [`safety::REFERENCE_YELLOW`] — including
/// [`safety::CAUTION_AMBER`]. Checking the segment closes the corridor
/// per-color distances leave open between neighbouring safety hues.
fn distance_to_warning_segment(c: Rgba8) -> u8 {
    c.r.abs_diff(255).max(c.b)
}

/// The four safety hues a themable color must keep its distance from.
/// [`safety::ANNUNCIATION_WHITE`] is deliberately absent: primary
/// symbology is legitimately white, and white advisory rows carry their
/// meaning by position in the stack, not by hue.
const SAFETY_HUES: [(&str, Rgba8); 4] = [
    ("failure red", safety::FAILURE_RED),
    ("caution amber", safety::CAUTION_AMBER),
    ("reference yellow", safety::REFERENCE_YELLOW),
    ("caution band", safety::BAND_CAUTION),
];

/// Integer Rec.601 luma, 0..=255.
fn luma(c: Rgba8) -> u8 {
    let weighted = 299u32 * u32::from(c.r) + 587 * u32::from(c.g) + 114 * u32::from(c.b);
    (weighted / 1000) as u8
}

/// `top` alpha-composited over an opaque `bottom` — the color the eye
/// actually meets where a translucent ground overlays the horizon.
fn composite(top: Rgba8, bottom: Rgba8) -> Rgba8 {
    let a = u32::from(top.a);
    let mix = |t: u8, b: u8| ((u32::from(t) * a + u32::from(b) * (255 - a)) / 255) as u8;
    Rgba8::rgb(
        mix(top.r, bottom.r),
        mix(top.g, bottom.g),
        mix(top.b, bottom.b),
    )
}

/// A theme that passed validation; the only theme drawing code accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidTheme(Theme);

impl ValidTheme {
    /// The shipped look, pre-validated.
    pub const DEFAULT: ValidTheme = ValidTheme(Theme::DEFAULT);

    /// Validates `theme` against the rules in the module doc.
    pub fn validate(theme: Theme) -> Result<ValidTheme, ThemeError> {
        for (field, color) in theme.all() {
            for (near, hue) in SAFETY_HUES {
                if max_channel_distance(color, hue) < SAFETY_DISTANCE_MIN {
                    return Err(ThemeError::ImitatesSafetyColor { field, near });
                }
            }
            if distance_to_warning_segment(color) < SAFETY_DISTANCE_MIN {
                return Err(ThemeError::ImitatesSafetyColor {
                    field,
                    near: "red-through-yellow segment",
                });
            }
            // Only the tape ground is legitimately translucent; every
            // other themable color paints at full opacity, so a theme
            // cannot fade symbology toward invisibility.
            let floor = if field == "tape_bg" {
                TAPE_ALPHA_MIN
            } else {
                255
            };
            if color.a < floor {
                return Err(ThemeError::AlphaBelowFloor {
                    field,
                    alpha: color.a,
                    floor,
                });
            }
        }
        // Primary symbology draws over the panel, the readout boxes,
        // both halves of the horizon, and the tapes; the tape ground is
        // measured as the eye meets it — composited at its declared
        // alpha over each horizon half — so a legal alpha cannot launder
        // a bright horizon past an RGB-only check.
        for (ground, color) in [
            ("panel_bg", theme.panel_bg),
            ("box_bg", theme.box_bg),
            ("sky", theme.sky),
            ("ground", theme.ground),
            ("tape_bg over sky", composite(theme.tape_bg, theme.sky)),
            (
                "tape_bg over ground",
                composite(theme.tape_bg, theme.ground),
            ),
        ] {
            let delta = luma(theme.primary).abs_diff(luma(color));
            if delta < CONTRAST_LUMA_MIN {
                return Err(ThemeError::InsufficientContrast { ground, delta });
            }
        }
        // The colored annunciation hues are unskinnable, but a theme
        // could still bury them in an equiluminant ground; hold them
        // apart from the grounds failure flags and dashes paint over.
        for (hue, color) in [
            ("failure red", safety::FAILURE_RED),
            ("caution amber", safety::CAUTION_AMBER),
        ] {
            for (ground, ground_color) in [("panel_bg", theme.panel_bg), ("box_bg", theme.box_bg)] {
                let delta = luma(color).abs_diff(luma(ground_color));
                if delta < SAFETY_CONTRAST_LUMA_MIN {
                    return Err(ThemeError::SafetyContrastBuried { hue, ground, delta });
                }
            }
        }
        if max_channel_distance(theme.gps, theme.radio_nav) < SAFETY_DISTANCE_MIN {
            return Err(ThemeError::IndistinctSourceColors);
        }
        Ok(ValidTheme(theme))
    }

    /// The validated colors.
    pub const fn get(&self) -> &Theme {
        &self.0
    }
}

#[cfg(test)]
mod tests;
