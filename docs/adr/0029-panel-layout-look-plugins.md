# ADR-0029: Panels, layout, and look are data-driven plugins over the scene and state contracts

- Status: Accepted
- Date: 2026-07-29
- Concrete contracts: [ADR-0033](0033-panel-registry-config-and-digest.md)

## Context

Operators will replace the PFD style, add a moving map, or restyle the whole
station; they may or may not have a gimbal camera or a joystick. The
instrument runtime ([ADR-0017](0017-instrument-display-runtime.md)) already
proves the load-bearing seam: the versioned scene-command IR is consumed by
two independent backends (browser canvas and the reference rasterizer) under
a conformance corpus. But everything above that seam is welded: the panel set
is enumerated in four layers (wasm exports, client script, health targets,
markup), the packed state ABI is closed, palette/geometry/glyphs are
compile-time constants, and some wire-adjacent feeder logic lives only in
client script contrary to
[ADR-0002](0002-cargo-workspace-portable-sans-io-core.md).

## Decision

- **A panel is a plugin over three stable contracts**: (1) an extensible
  **state-group contract** — typed signal groups with validity and age,
  declared per panel, replacing the closed fixed-layout ABI; (2) the
  **scene-command IR** ([ADR-0017](0017-instrument-display-runtime.md));
  (3) the **glyph vocabulary**. A panel remains a pure state→scene
  function; the shell enforces layer masks and budgets generically.
- **A panel registry replaces hard-coded enumeration.** A panel descriptor
  — identity, required layers, required state groups, preferred aspect —
  is the single source every shell (web, native) consumes for wiring,
  health tracking, and layout slots. A required group that is unfed
  renders honest Missing status, never a blank or a default.
- **All panels share one data model.** The state vocabulary, the navigation
  solution ([ADR-0024](0024-navigation-authority-boundary.md)), navdata
  snapshots (Communicate), and signed terrain packages (`pilotage-svs-db`)
  feed every panel through the same group contract; a replacement PFD and
  a moving map consume the same groups, not private feeds.
- **Look and layout are data, with a safety-fixed floor.** Theming inputs
  (palette, proportions, glyph pack selection) are validated data. A
  defined set of visual attributes is never skinnable: failure pages,
  flag colors, alert semantics, and required-layer isolation. Panel
  placement and selection are declarative configuration consumed by web
  and native shells alike.
- **Feeder logic is shared core.** Ingress gating, coherence decisions,
  and derivations that interpret wire or measurement semantics live in
  sans-IO core crates driven by each platform
  ([ADR-0002](0002-cargo-workspace-portable-sans-io-core.md)); client
  script holds no wire- or measurement-interpreting logic.
- **Foreign panels are admitted by conformance, not trust.** A harness
  validates a plugin panel's scenes — layer contract, budgets, glyph
  vocabulary coverage, and honest-status rendering under injected
  Missing/Stale/Failed inputs — the inverse of the existing backend
  conformance, before a panel may join an operational layout.

## Consequences

- Which attributes are safety-fixed versus skinnable is an explicit
  assurance decision recorded with the theming schema, not an emergent
  property of what happens to be a constant.
- The registry and extensible state groups are substantial, staged work:
  the wasm boundary, the shells, and the health model all consume the
  descriptor instead of literals.
- The moving map becomes buildable: its data path exists once navigation
  solution and navdata groups flow through the group contract.
- iPadOS and native stations reuse panels unchanged; platform work is
  shell and ports, not panel code.
- Plugin isolation and versioning (how a third-party panel is loaded,
  sandboxed, and version-matched to the contracts) is an open question
  tracked below.

## Open questions

- Plugin packaging and isolation: separate wasm modules against a frozen
  import contract versus out-of-process scene producers over a channel.
- The assurance bar for third-party panels in operational (non-simulation)
  deployments.

## Alternatives considered

- **Fork-and-recompile as the customization model:** rejected; every look
  or panel variant becomes a divergent build of safety-adjacent code.
- **A widget toolkit with arbitrary drawing:** rejected; the frozen scene
  vocabulary and glyph discipline are what make honest-status rendering
  and conformance checking tractable.
