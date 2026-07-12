//! Space and the punctuation the instruments draw: hyphen/minus, period,
//! slash (the `SIM / NOT FOR FLIGHT` separator), and the degree sign.

use crate::glyph::Glyph;

/// Symbol glyphs, in canonical (id) order.
pub(crate) const SYMBOLS_ARR: [Glyph; 5] = [
    Glyph::new(' ', [0, 0, 0, 0, 0, 0, 0]),
    Glyph::new('-', [0, 0, 0, 0b01110, 0, 0, 0]),
    Glyph::new('.', [0, 0, 0, 0, 0, 0b00110, 0b00110]),
    Glyph::new(
        '/',
        [
            0b00001, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b10000,
        ],
    ),
    Glyph::new('°', [0b01100, 0b10010, 0b10010, 0b01100, 0, 0, 0]),
];

/// Borrowed view of [`SYMBOLS_ARR`].
pub(crate) const SYMBOLS: &[Glyph] = &SYMBOLS_ARR;
