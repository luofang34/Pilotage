//! The mandatory character vocabulary the glyph pack must cover.
//!
//! [`PANEL_VOCABULARY`] is derived from the strings the
//! `pilotage-instrument-panels` crate emits: every `text(...)` literal and
//! every `fmt_label!` template across the PFD and HSI. Digits come from the
//! tape, rose, and readout numerals; `-` from dashes and negative readouts;
//! `.` and `°` from the distance and heading/course/wind labels; the
//! uppercase letters from the fixed labels (`IAS`, `ALT`, `ATT`, `GS`,
//! `WIND`, `DIST NM`, `CRS`, the `N`/`E`/`S`/`W` rose marks, and the
//! `V`/`G` deviation tags); and `k`/`t` from the `kt` speed unit.
//!
//! [`FLAG_VOCABULARY`] adds the simulation and conformality labels that
//! each surface must render for requirements AIR-FLAG-007 and AIR-BAS-001;
//! they draw from the full uppercase set plus the slash separator.
//!
//! A completeness test resolves every character in these lists against the
//! pack. Wiring a compile-time dependency from the panels crate onto this
//! one is deferred to the integration change; until then the vocabulary is
//! pinned by this static list rather than by re-parsing the panels source.

/// Characters the PFD and HSI panels emit, in canonical order.
pub const PANEL_VOCABULARY: &[char] = &[
    ' ', '-', '.', '°', //
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', //
    'A', 'C', 'D', 'E', 'G', 'I', 'L', 'M', 'N', 'R', 'S', 'T', 'V', 'W', //
    'k', 't',
];

/// Representative label strings the panels draw; the completeness test
/// requires every character of each to resolve.
pub const PANEL_STRINGS: &[&str] = &[
    "IAS", "ALT", "ATT", "GS 0kt", "WIND ---", "DIST NM", "CRS", "N", "E", "S", "W", "V", "G",
    "---", "--.-", "---°", "360°", "-100",
];

/// Simulation and conformality labels every surface must render
/// (AIR-FLAG-007, AIR-BAS-001).
pub const FLAG_VOCABULARY: &[&str] = &[
    "SIM / NOT FOR FLIGHT",
    "HUD-SIM",
    "NON-CONFORMAL / NOT A HUD",
];
