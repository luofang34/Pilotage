# Controlled reproducible glyph pack

This document records the authorship, license, deterministic build inputs, and
content hash of the instrument glyph pack in
`crates/pilotage-instrument-glyphs`. It is a deterministic engineering contract
for the simulator and reference backends; it does not claim certification
credit.

The pack removes operating-system font substitution from mandatory instrument
symbology (ADR-0017): panels emit `text` commands, and a backend renders them
from this controlled asset instead of an installed system font.

## Authorship and license

Every glyph is an original 5×7 bitmap authored for this project directly in
Rust source, as bit matrices in
`crates/pilotage-instrument-glyphs/src/font/`. The pack embeds **no
third-party font data** — no downloaded, system, or externally licensed font
contributes any pixel — so it carries no external font license obligation and
is distributed under the repository's own terms. Selecting a licensed outline
font later is an upgrade path (see below), not a dependency today.

## Glyph vocabulary

The mandatory vocabulary is derived from what the panels actually draw, not
guessed. Every `text(...)` literal and every `fmt_label!` template in
`crates/pilotage-instrument-panels` was enumerated:

- **Digits `0`–`9`** — tape, rose, and readout numerals.
- **`-`** — dash runs (`---`, `--.-`, `---°`) and negative readouts.
- **`.`** — the distance readout and `--.-`.
- **`°`** — heading, course, and wind labels.
- **space** — `WIND ---`, `DIST NM`, `GS …kt`.
- **Uppercase `A C D E G I L M N R S T V W`** — the fixed labels `IAS`, `ALT`,
  `ATT`, `GS`, `WIND`, `DIST NM`, `CRS`, the `N`/`E`/`S`/`W` rose marks, and
  the `V`/`G` vertical-deviation tags.
- **Lowercase `k t`** — the `kt` speed unit.

This set is pinned as `PANEL_VOCABULARY`. The pack additionally provides the
full uppercase `A`–`Z`, lowercase `a`–`z`, and the slash `/` so that the
simulation and conformality labels required by
[`AIR-FLAG-007`](requirements.md#air-flag-007) and
[`AIR-BAS-001`](requirements.md#air-bas-001) — `SIM / NOT FOR FLIGHT`,
`HUD-SIM`, `NON-CONFORMAL / NOT A HUD` — and unit tokens such as `ft`, `fpm`,
`hPa`, and `nm` render without a later vocabulary change. Completeness over
both `PANEL_VOCABULARY` and the flag labels is enforced by tests.

## Manifest contract

Both renderers consume one `GlyphManifest`, so they agree on:

- **version** — the manifest format version (currently `1`).
- **cell** — the glyph matrix size, `5×7` pixels.
- **advance** — monospace horizontal advance, `6` cell columns.
- **baseline** — rows from the cell top to the text baseline, `7` (no
  sub-baseline descenders in this pack).
- **glyph id** — a stable `u16` assigned by canonical order; the same id
  denotes the same glyph on either renderer.
- **bitmap** — seven `u8` rows; the low five bits are pixels, bit 4 leftmost.

Text metrics remain backend-owned (ADR-0017): the manifest publishes advance
and baseline for backends that lay out by metric, but panel layouts position
text by anchor and do not depend on precise glyph geometry.

## Deterministic build inputs

The content hash is a function of exactly these inputs, and nothing else — no
timestamps, no random values, no network fetch, no host font, no environment:

1. The glyph bit matrices in `src/font/symbols.rs`, `digits.rs`,
   `letters_upper.rs`, and `letters_lower.rs`.
2. The geometry constants `CELL_W = 5`, `CELL_H = 7`, `ADVANCE = 6`,
   `BASELINE = 7`, `GLYPH_MANIFEST_VERSION = 1`.
3. The frozen class concatenation order `SYMBOLS, DIGITS, UPPER, LOWER`, which
   assigns glyph ids.
4. The canonical serialization layout below.
5. SHA-256 (FIPS 180-4).

### Canonical serialization

```text
header (8 bytes):
  [version u16 LE][cell_w u8][cell_h u8][advance u8][baseline u8][count u16 LE]
per glyph, in id order:
  [char u32 LE][advance u8][row0 u8]…[row6 u8]
```

The content hash is the SHA-256 of this byte sequence. It is computed at build
time by a `const fn` over the shipped glyphs, so verification compares live
data against a hash the build itself produced — there is no hand-entered
digest in the library. Reproducibility is a test: the canonical form is built
twice and compared byte-for-byte, and the hash is recomputed and compared.

## Recorded content hash

For the current shipped pack (67 glyphs, 812 canonical bytes, manifest
version 1):

```text
sha256 = 281eef6229feee417c7090d8c8ea79489c017cd1c02fc7234876b2a64a532158
```

This value is pinned by a test; any glyph, geometry, or ordering change fails
that test until this record is updated in the same change.

## Verification and fail-closed behavior

`GlyphManifest::verify` checks, in order: geometry in range, every glyph
well-formed (non-zero advance, no pixels outside the cell), every mandatory
character present, and the recomputed content hash equal to the recorded one.
Each failure returns a specific typed reason
([`AIR-OUT-004`](requirements.md#air-out-004),
[`AIR-BAS-007`](requirements.md#air-bas-007)):

- a missing mandatory glyph → `MissingGlyph`;
- a malformed glyph → `InvalidGlyph`;
- out-of-range geometry → `InvalidGeometry`;
- a wrong or corrupt hash → `ContentHashMismatch`.

Lookup fails closed too: `glyph(ch)` for an uncovered character returns
`MissingGlyph`, never a substitute or a system glyph. A backend is expected to
run `verify` and declare itself ready only after it passes.

## Deferred and upgrade path

- **Browser and panels wiring** is deferred to a later integration change: the
  browser backend will load and verify this asset before declaring ready, and
  a compile-time dependency from `pilotage-instrument-panels` onto this crate
  will let the panel vocabulary be checked against the source strings directly.
  Until then the vocabulary is pinned by the static `PANEL_VOCABULARY` list.
- **Richer outline font** is a separately-licensed upgrade behind this same
  manifest contract. It would change the geometry fields and bitmap/outline
  representation, bump `GLYPH_MANIFEST_VERSION`, and produce a new recorded
  hash — consumers keep the same `verify`, ids, and metrics contract.
