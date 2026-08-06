//! Theme validation: the shipped look passes, hostile themes are
//! refused, and the themable set stays equal to the palette defaults so
//! adopting the theme path repaints nothing.

#![allow(clippy::expect_used, clippy::panic)]

use super::{TAPE_ALPHA_MIN, Theme, ThemeError, ValidTheme};
use crate::{palette, safety};
use pilotage_instrument_scene::Rgba8;

#[test]
fn the_default_theme_validates() {
    ValidTheme::validate(Theme::DEFAULT).expect("shipped look must pass its own rules");
    assert_eq!(*ValidTheme::DEFAULT.get(), Theme::DEFAULT);
}

#[test]
fn the_default_theme_is_the_palette() {
    let theme = Theme::DEFAULT;
    assert_eq!(theme.sky, palette::SKY);
    assert_eq!(theme.ground, palette::GROUND);
    assert_eq!(theme.primary, palette::WHITE);
    assert_eq!(theme.panel_bg, palette::BLACK);
    assert_eq!(theme.tape_bg, palette::TAPE_BG);
    assert_eq!(theme.box_bg, palette::BOX_BG);
    assert_eq!(theme.outline, palette::GREY);
    assert_eq!(theme.gps, palette::MAGENTA);
    assert_eq!(theme.radio_nav, palette::GREEN);
    assert_eq!(theme.selection, palette::CYAN);
    assert_eq!(theme.band_normal, palette::BAND_GREEN);
}

#[test]
fn the_palette_safety_constants_are_the_safety_module() {
    assert_eq!(palette::RED, safety::FAILURE_RED);
    assert_eq!(palette::AMBER, safety::CAUTION_AMBER);
    assert_eq!(palette::YELLOW, safety::REFERENCE_YELLOW);
    assert_eq!(palette::BAND_YELLOW, safety::BAND_CAUTION);
    assert_eq!(palette::WHITE, safety::ANNUNCIATION_WHITE);
}

#[test]
fn a_red_sky_is_rejected_as_imitation() {
    let hostile = Theme {
        sky: Rgba8::rgb(250, 4, 4),
        ..Theme::DEFAULT
    };
    assert_eq!(
        ValidTheme::validate(hostile),
        Err(ThemeError::ImitatesSafetyColor {
            field: "sky",
            near: "failure red",
        })
    );
}

#[test]
fn an_amber_selection_is_rejected_as_imitation() {
    let hostile = Theme {
        selection: Rgba8::rgb(240, 180, 30),
        ..Theme::DEFAULT
    };
    assert_eq!(
        ValidTheme::validate(hostile),
        Err(ThemeError::ImitatesSafetyColor {
            field: "selection",
            near: "caution amber",
        })
    );
}

#[test]
fn the_exact_reference_yellow_is_rejected_as_imitation() {
    // A selection painted exactly the fixed aircraft-reference hue
    // would make pilot bugs indistinguishable from the reference
    // symbol.
    let hostile = Theme {
        selection: safety::REFERENCE_YELLOW,
        ..Theme::DEFAULT
    };
    assert!(matches!(
        ValidTheme::validate(hostile),
        Err(ThemeError::ImitatesSafetyColor {
            field: "selection",
            ..
        })
    ));
}

#[test]
fn the_red_to_yellow_corridor_is_closed() {
    // A saturated warning orange sits 88 from both FAILURE_RED and
    // CAUTION_AMBER — past the per-color floor — but ON the warning
    // hue segment, and must still be refused.
    let hostile = Theme {
        gps: Rgba8::rgb(255, 88, 0),
        ..Theme::DEFAULT
    };
    assert_eq!(
        ValidTheme::validate(hostile),
        Err(ThemeError::ImitatesSafetyColor {
            field: "gps",
            near: "red-through-yellow segment",
        })
    );
}

#[test]
fn low_contrast_primary_is_rejected_against_each_ground() {
    let murky = Theme {
        primary: Rgba8::rgb(60, 60, 60),
        ..Theme::DEFAULT
    };
    assert_eq!(
        ValidTheme::validate(murky),
        Err(ThemeError::InsufficientContrast {
            ground: "panel_bg",
            delta: 60,
        })
    );
    // The box ground is checked in its own right, not shadowed by the
    // panel ground.
    let grey_boxes = Theme {
        box_bg: Rgba8::rgb(200, 200, 200),
        ..Theme::DEFAULT
    };
    assert_eq!(
        ValidTheme::validate(grey_boxes),
        Err(ThemeError::InsufficientContrast {
            ground: "box_bg",
            delta: 55,
        })
    );
    // And the tape ground: white digits sit directly on it. The tape is
    // measured composited over the horizon, so an opaque wash fails
    // against the sky half first.
    let washed_tapes = Theme {
        tape_bg: Rgba8::rgba(250, 250, 250, 255),
        ..Theme::DEFAULT
    };
    assert_eq!(
        ValidTheme::validate(washed_tapes),
        Err(ThemeError::InsufficientContrast {
            ground: "tape_bg over sky",
            delta: 5,
        })
    );
}

#[test]
fn a_bright_sky_is_rejected_for_primary_contrast() {
    // The horizon halves are grounds primary draws over (pitch ladder,
    // roll pointer, horizon line); a near-white sky would erase them.
    let hostile = Theme {
        sky: Rgba8::rgb(250, 250, 250),
        ..Theme::DEFAULT
    };
    assert_eq!(
        ValidTheme::validate(hostile),
        Err(ThemeError::InsufficientContrast {
            ground: "sky",
            delta: 5,
        })
    );
}

#[test]
fn a_translucent_tape_over_a_legal_sky_is_measured_composited() {
    // Sky at luma 150 passes the plain check, and the tape's alpha 96
    // is at its floor — but the composite the eye meets is too bright
    // for white digits. An RGB-only check would wave this through.
    let hostile = Theme {
        sky: Rgba8::rgb(150, 150, 150),
        tape_bg: Rgba8::rgba(200, 200, 200, 96),
        ..Theme::DEFAULT
    };
    assert_eq!(
        ValidTheme::validate(hostile),
        Err(ThemeError::InsufficientContrast {
            ground: "tape_bg over sky",
            delta: 87,
        })
    );
}

#[test]
fn a_ground_equiluminant_with_failure_red_is_rejected() {
    // luma(0,130,0) == luma(FAILURE_RED): the "value not trustworthy"
    // dashes would have zero luminance contrast against their own box,
    // in the exact red-on-green pairing color deficiency cannot separate.
    let hostile = Theme {
        box_bg: Rgba8::rgb(0, 130, 0),
        ..Theme::DEFAULT
    };
    assert_eq!(
        ValidTheme::validate(hostile),
        Err(ThemeError::SafetyContrastBuried {
            hue: "failure red",
            ground: "box_bg",
            delta: 0,
        })
    );
}

#[test]
fn a_panel_that_buries_caution_amber_is_rejected() {
    let hostile = Theme {
        panel_bg: Rgba8::rgb(100, 180, 100),
        ..Theme::DEFAULT
    };
    assert!(matches!(
        ValidTheme::validate(hostile),
        Err(ThemeError::SafetyContrastBuried {
            hue: "caution amber",
            ground: "panel_bg",
            ..
        })
    ));
}

#[test]
fn every_colored_safety_constant_is_screened_or_exempt() {
    // SAFETY_HUES is a hand-maintained list; this keeps it honest
    // against the safety module. A new safety constant must join the
    // imitation screen or the documented white exemption.
    let screened = super::SAFETY_HUES;
    let exempt = [safety::ANNUNCIATION_WHITE];
    for (name, color) in [
        ("FAILURE_RED", safety::FAILURE_RED),
        ("CAUTION_AMBER", safety::CAUTION_AMBER),
        ("REFERENCE_YELLOW", safety::REFERENCE_YELLOW),
        ("BAND_CAUTION", safety::BAND_CAUTION),
        ("ANNUNCIATION_WHITE", safety::ANNUNCIATION_WHITE),
    ] {
        assert!(
            screened.iter().any(|(_, hue)| *hue == color) || exempt.contains(&color),
            "{name} escapes the imitation screen"
        );
    }
}

#[test]
fn transparent_colors_are_rejected_everywhere() {
    // Fully transparent primary symbology passes an RGB-only contrast
    // check while rendering invisible; the alpha floor refuses it.
    let ghost_primary = Theme {
        primary: Rgba8::rgba(255, 255, 255, 0),
        ..Theme::DEFAULT
    };
    assert_eq!(
        ValidTheme::validate(ghost_primary),
        Err(ThemeError::AlphaBelowFloor {
            field: "primary",
            alpha: 0,
            floor: 255,
        })
    );
    let seethrough_panel = Theme {
        panel_bg: Rgba8::rgba(0, 0, 0, 200),
        ..Theme::DEFAULT
    };
    assert_eq!(
        ValidTheme::validate(seethrough_panel),
        Err(ThemeError::AlphaBelowFloor {
            field: "panel_bg",
            alpha: 200,
            floor: 255,
        })
    );
    let seethrough_box = Theme {
        box_bg: Rgba8::rgba(0, 0, 0, 200),
        ..Theme::DEFAULT
    };
    assert_eq!(
        ValidTheme::validate(seethrough_box),
        Err(ThemeError::AlphaBelowFloor {
            field: "box_bg",
            alpha: 200,
            floor: 255,
        })
    );
    let ghost_tape = Theme {
        tape_bg: Rgba8::rgba(20, 20, 20, 40),
        ..Theme::DEFAULT
    };
    assert_eq!(
        ValidTheme::validate(ghost_tape),
        Err(ThemeError::AlphaBelowFloor {
            field: "tape_bg",
            alpha: 40,
            floor: TAPE_ALPHA_MIN,
        })
    );
}

#[test]
fn indistinct_source_colors_are_rejected() {
    let monosource = Theme {
        radio_nav: Rgba8::rgb(255, 40, 255),
        ..Theme::DEFAULT
    };
    assert_eq!(
        ValidTheme::validate(monosource),
        Err(ThemeError::IndistinctSourceColors)
    );
}
