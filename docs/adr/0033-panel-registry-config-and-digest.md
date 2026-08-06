# ADR-0033: Panel registry as composed data, key-TLV configuration, theme floors, and the scene digest

- Status: Accepted
- Date: 2026-08-06

## Context

[ADR-0029](0029-panel-layout-look-plugins.md) decided that panels are
data-driven plugins over the state-group, scene-IR, and glyph contracts.
This record fixes the concrete shapes that decision left open: how a
shell learns what panels exist, how per-panel options cross the
wasm/FFI boundary, which theme constraints are the safety floor, and
what single invariant proves a panel renders identically on every
shell. The state-group side is already concrete (tagged-group state ABI
v6; absence resolves Missing by construction).

## Decision

- **The registry is plain data composed by each shell.** A
  `PanelDescriptor` (in `pilotage-instrument-registry`) carries
  identity, title, required layers, required state groups, design
  frame, background capability, config schema, honest-status group
  regions, panel-contributed extreme states, the pinned raster
  baseline, and the draw entry point. `Registry::new` validates the
  composition at init — malformed ids, undefined layer bits, degenerate
  frames, unordered schemas, and dishonest group regions fail the shell
  before anything draws. There is no link-time registration magic: an
  out-of-repo panel registers by being listed in the shell's descriptor
  slice, so what a station displays is reviewable configuration.
- **Configuration is a bounded key-TLV blob.** `[key u16 LE][len u16
  LE][payload]`, strictly ascending keys, at most 256 bytes, crossing
  every boundary as one byte slice. A key outside the panel's declared
  schema is rejected, never skipped: silently ignoring an option a
  caller believes is set would misstate what the panel displays.
  Well-known keys start at `0x0001`; out-of-repo panels take `0x8000`
  and up.
- **SVS is accept-and-cede.** `BackgroundMode::Svs { viewport, quality }`
  is validated configuration a PFD carries today and draws exactly as
  `BackgroundMode::None` — the Background band is ceded, and nothing
  above that band may depend on the choice (pinned by a
  byte-equivalence test). The SVS renderer, when it lands, consumes the
  ceded band; no panel changes shape.
- **Theme floors are assurance decisions, recorded here.** A themable
  color within max-channel distance 64 of any warning hue or of the
  red-through-yellow segment is refused (white is exempt: primary
  symbology is legitimately white, and white advisory rows signal by
  stack position). Primary symbology holds 96 luma against every ground
  it draws over, with the translucent tape ground measured composited
  over each horizon half at its declared alpha. The colored
  annunciation hues hold 64 luma against the panel and box grounds —
  below the primary floor deliberately, because failure red holds only
  76 against the shipped black; the rule defends against equiluminant
  burial (the red-on-green deficiency case), not ordinary legibility.
  Every themable color paints at full alpha except the tape ground with
  its own floor of 96. Tightening any floor is a reviewable change to
  these numbers, not a code archaeology exercise.
- **One digest proves cross-shell identity.** The migration invariant
  is a streaming SHA-256 over a domain separator, the scene/state
  contract versions, and, per registered panel: the role-tagged,
  length-prefixed panel id with the contract-relevant descriptor
  fields (required layers, required groups, design frame, background
  capability, config schema), then per corpus state — the shared
  canonical set plus that panel's own extreme states — the role-tagged
  state id and emitted scene bytes. Everything draws with the empty
  config and no alerts, so the digest is invariant to SVS by
  construction; theme independence holds because panels take no theme
  parameter at this boundary. Shells report the same digest or they
  are not showing the same instruments; pixel hashes remain
  per-backend rasterizer regression tests, not the cross-shell
  contract.

## Consequences

- Adding a panel touches the registry composition and the shell's
  descriptor list — not shell match ladders, health tables, or markup.
- The admission harness gets its matrix from the descriptor
  (`required_groups` × withholding, `group_regions` for honest-status
  checks), so a dishonest panel is refused mechanically.
- The digest moves exactly once per deliberate contract change, with
  the review note saying why; an unexplained digest move is a defect.
- Config keys form a small registry of their own; retiring a key means
  rejecting it (schemas drop it), never silently ignoring it.

## Alternatives considered

- **Link-time plugin registration (linkme/inventory):** rejected — what
  a station displays must be reviewable data, not an emergent property
  of what happened to link.
- **Per-panel typed config across the boundary:** rejected — every new
  option would grow the wasm/FFI surface; one validated blob keeps the
  boundary append-only.
- **Pixel hashes as the cross-shell invariant:** rejected — they bind a
  rasterizer, not the panel contract; two conformant backends
  legitimately rasterize differently.
