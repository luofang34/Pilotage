//! Lowercase Latin letters `a`–`z`.
//!
//! The panels emit `k` and `t` today (the `kt` speed unit); the full set
//! is provided so unit tokens such as `ft`, `fpm`, `hPa`, and `nm` render
//! without a later vocabulary change. Cells are seven rows tall with no
//! sub-baseline descenders; tails of `g`/`j`/`p`/`q`/`y` sit within the
//! cell, a limitation an outline-font upgrade would lift.

use crate::glyph::Glyph;

/// Lowercase glyphs, in canonical (id) order.
pub(crate) const LOWER_ARR: [Glyph; 26] = [
    Glyph::new('a', [0, 0, 0b01110, 0b00001, 0b01111, 0b10001, 0b01111]),
    Glyph::new(
        'b',
        [
            0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b10001, 0b11110,
        ],
    ),
    Glyph::new('c', [0, 0, 0b01110, 0b10001, 0b10000, 0b10001, 0b01110]),
    Glyph::new(
        'd',
        [
            0b00001, 0b00001, 0b01111, 0b10001, 0b10001, 0b10001, 0b01111,
        ],
    ),
    Glyph::new('e', [0, 0, 0b01110, 0b10001, 0b11111, 0b10000, 0b01110]),
    Glyph::new(
        'f',
        [
            0b00110, 0b01001, 0b01000, 0b11100, 0b01000, 0b01000, 0b01000,
        ],
    ),
    Glyph::new(
        'g',
        [0, 0b01111, 0b10001, 0b10001, 0b01111, 0b00001, 0b01110],
    ),
    Glyph::new(
        'h',
        [
            0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b10001, 0b10001,
        ],
    ),
    Glyph::new(
        'i',
        [0b00100, 0, 0b01100, 0b00100, 0b00100, 0b00100, 0b01110],
    ),
    Glyph::new(
        'j',
        [0b00010, 0, 0b00110, 0b00010, 0b00010, 0b10010, 0b01100],
    ),
    Glyph::new(
        'k',
        [
            0b10000, 0b10000, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010,
        ],
    ),
    Glyph::new(
        'l',
        [
            0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
    ),
    Glyph::new('m', [0, 0, 0b11010, 0b10101, 0b10101, 0b10001, 0b10001]),
    Glyph::new('n', [0, 0, 0b11110, 0b10001, 0b10001, 0b10001, 0b10001]),
    Glyph::new('o', [0, 0, 0b01110, 0b10001, 0b10001, 0b10001, 0b01110]),
    Glyph::new(
        'p',
        [0, 0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000],
    ),
    Glyph::new(
        'q',
        [0, 0b01111, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001],
    ),
    Glyph::new('r', [0, 0, 0b10110, 0b11001, 0b10000, 0b10000, 0b10000]),
    Glyph::new('s', [0, 0, 0b01111, 0b10000, 0b01110, 0b00001, 0b11110]),
    Glyph::new(
        't',
        [
            0b01000, 0b01000, 0b11100, 0b01000, 0b01000, 0b01001, 0b00110,
        ],
    ),
    Glyph::new('u', [0, 0, 0b10001, 0b10001, 0b10001, 0b10001, 0b01111]),
    Glyph::new('v', [0, 0, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100]),
    Glyph::new('w', [0, 0, 0b10001, 0b10001, 0b10101, 0b10101, 0b01010]),
    Glyph::new('x', [0, 0, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001]),
    Glyph::new(
        'y',
        [0, 0b10001, 0b10001, 0b10001, 0b01111, 0b00001, 0b01110],
    ),
    Glyph::new('z', [0, 0, 0b11111, 0b00010, 0b00100, 0b01000, 0b11111]),
];

/// Borrowed view of [`LOWER_ARR`].
pub(crate) const LOWER: &[Glyph] = &LOWER_ARR;
