# ADR-0017: Instrument display runtime as a no_std sans-IO crate family emitting a versioned scene-command IR

- Status: Accepted
- Date: 2026-07-09

## Context

Pilotage needs an instrument display runtime: PFD, HSI, six-pack, and later
engine/map pages, rendered in the browser beside video today, on native and
embedded panels tomorrow. The defining requirement is that these are **display
surfaces of one underlying model** — a unified aircraft state, navigation
state, validity model, and display model — not independent programs, and that
the component library binds to **no specific graphics backend**: components
emit abstract drawing commands; wgpu, software raster, Canvas2D, or an
embedded framebuffer render them.

Three findings shape the decision:

- **The certified world already validated this split.** ARINC 661 separates a
  cockpit display server from user applications precisely so presentation and
  logic can change and be validated independently; its load-bearing property
  is a *finite, versioned* widget/command vocabulary over a thin protocol.
  Betaflight's OSD reaches the same shape at hobby scale (elements call a tiny
  draw vtable; a character chip or HD goggles render it). The pattern to copy
  is the frozen small vocabulary, not any particular widget set.
- **The Rust space is greenfield.** There is no existing Rust
  EFIS/PFD/instrument runtime — only substrate (egui, wgpu, Slint,
  embedded-graphics, rust-mavlink). The instrument layer, validity model, and
  display/backend split must be built, and nothing forces another project's
  std/alloc assumptions on us.
- **Aviate deliberately does no UI**, and its kernel exposes exactly the state
  a display consumes (quaternion attitude, NED position/velocity, per-field
  validity flags, quality enum). The display runtime is the missing half of
  that contract, and it must eventually run on the same class of embedded
  target Aviate flies on.

The immediate question is where this code lives and what its core abstractions
are.

## Decision

### Same repository, as a leaf crate family that could stand alone

The runtime lives in this workspace as a sans-IO crate family, the same
shape as `pilotage-timing`: **leaf crates with no dependency on
`pilotage-protocol`, prost, or any transport**. Pilotage supplies everything
around it — WebTransport, the session host, the Gazebo sim, the web client,
CI gates — and ADR-0002 already reserved the browser seam ("the same Rust
core compiled to wasm"); this family is that plan realized. A separate repo
would buy independence we do not need yet at the cost of the integration we
need immediately. Because the core crates are leaves, extraction later (for
example to serve an Aviate onboard display directly) is a `git mv`, not a
divorce; a thin bridge module owns the mapping from Pilotage wire telemetry
into instrument state, and it is the only place the two vocabularies meet.

### Three crates, strictly `#![no_std]`, no alloc

| Crate | Owns |
|---|---|
| `pilotage-instrument-state` | Unified aircraft/nav/selection state, per-signal validity, unit conversions, quaternion→attitude derivations |
| `pilotage-instrument-scene` | The draw-command IR: command enum, fixed-capacity encoder/decoder over caller-provided buffers, versioned little-endian binary encoding |
| `pilotage-instrument-panels` | The instruments: PFD, HSI, six-pack as pure functions `(state, layout) → scene commands` |

These are the workspace's first strictly `#![no_std]` crates (existing core
crates are IO-free but assume alloc). They use `f32` and `libm` only, no
allocator: the scene encoder writes into a caller-provided `&mut [u8]` and
returns `Err` on overflow rather than growing. The workspace lint wall
(forbid `unsafe_code`; deny `unwrap_used`, `expect_used`, `panic`) applies
unchanged; `no_std` + no-alloc is enforced in CI by compiling the family for
`thumbv7em-none-eabihf`, so the claim cannot rot into a comment.

### The scene IR is the backend contract

Panels never draw. They emit a flat, ordered command list — the
immediate-mode family (egui draw lists, Skia pictures), not a retained tree —
because a per-frame command buffer is the simplest thing that serializes
across a WASM boundary, a network, or a DMA channel, and instruments redraw
whole panels at fixed cadence anyway, so retained-tree diffing buys nothing.

- The vocabulary is deliberately small and **versioned** (a header byte leads
  every encoded scene): transform save/restore/translate/rotate, clip rect,
  line, polyline, polygon, rect, arc, circle, text with anchor, solid RGBA
  paints. Growth is by appending opcodes, never redefining them.
- Decoders MUST treat an unknown opcode as a counted, logged skip of that
  command — never a hard failure — mirroring ADR-0016's unknown-FourCC rule,
  so an older backend degrades gracefully against a newer core.
- Text metrics are backend-owned: commands carry position, size, and anchor;
  the backend chooses the font. Instrument layouts must therefore not depend
  on precise glyph metrics.

The first backend is the browser: the panels crate compiles to
`wasm32-unknown-unknown`, and wasm-bindgen exposes an explicit
`InstrumentRuntime` resource owned by its JavaScript caller. Each resource
owns fixed state/scene buffers, configuration, and wrapping generations; the
Rust boundary has no module-level mutable state. The caller writes packed
state into the resource's linear-memory range, calls one `render_result`, and
receives status, scene length, and generation in one packed value before a
small JS module interprets the scene onto Canvas2D. wgpu, software raster,
and embedded framebuffer backends consume the identical encoding later.

### Validity is first-class state, not an afterthought

Every signal group carries a status — `Valid`, `Degraded`, `Stale`,
`Missing`, `Failed` — resolved sans-IO from per-group freshness timestamps
against a caller-supplied `now` and a staleness policy (the pattern of
ADR-0009, applied to display data). Panels MUST render non-`Valid` state
distinctly (red-X, dashes, flagged) and MUST NOT silently hold last-good
values; a data source that never provides a signal (no airspeed sensor)
yields an honest `Missing`, not a fabricated number. This is the single
biggest lesson from surveying the field: every serious system (dual-ADAHRS
comparators, Stratux invalid sentinels) treats validity as data, and the
reference hobby implementations that skip it (pyG5's lone avionics-on flag)
cannot express "this one tape is lying to you."

### Reserved, unbuilt extension points

Phase 1 implements 2D PFD, HSI, and six-pack only. Three seams are reserved
so SVS and embedded degradation never require re-architecture:

- **Background slot**: the PFD takes a background mode (`Horizon` now;
  `Svs { viewport }` reserved) so a synthetic-vision layer slots under the
  symbology without touching tapes or ladder. SVS fidelity is a
  backend-class decision (survey lesson: every vendor scales SVS to the
  processor), so the slot carries a quality tier, not a renderer.
- **Overlay layers**: implemented — see "The scene-layer and safety
  compositor contract" below.
- **Data-source abstraction**: instrument state is fed by writers, never by
  transports; local-sensor, bus, and network sources all converge on the same
  state struct (the thin-display vs. smart-display axis stays open).

### The scene-layer and safety compositor contract

Scene commands partition into six bounded, named, z-ordered criticality
bands, delimited by begin/end layer markers in the reserved 0x50 opcode
space: `Background` (0), `Attitude` (1), `Tapes` (2), `Guidance` (3),
`Annunciation` (4), `Failure` (5). The contract exists so optional
background imagery can never cover, suppress, or prevent primary flight
information, warnings, or failure indications (AC 25-11B's
mixed-criticality display concern, as a design input — not a compliance
claim). The byte-level contract is specified in the
[scene-layer protocol](../instruments/scene-layer-protocol.md).

- Encoding order **is** z-order: layers appear at most once each, in
  strictly ascending id order, unnested, with every drawing command
  inside exactly one layer. A validated scene therefore cannot paint
  background over a critical band, on any conforming backend.
- Every layer carries a mandatory outer Save/Restore envelope immediately
  inside its markers. No command may sit outside that envelope. It isolates
  transform, clip, and paint state even on an older backend that skips the
  markers themselves.
- Frames are bounded: per-layer command count, graphics-state stack depth,
  and scene byte budgets are compile-time constants in
  `pilotage-instrument-scene::layer`. `validate_layers` enforces these with
  the duplicate, ordering, nesting, state-isolation, and framing rules.
- Version policy: unknown *opcodes* inside a layer remain counted skips;
  an unknown *layer id* fails the whole frame, because content whose
  criticality cannot be placed must not be painted. Layer marker payloads
  are exactly one byte. Growing the layer vocabulary or marker shape
  therefore requires a scene format version bump.
- The SVS/raster boundary: backend-owned raster or depth imagery (a
  hypothetical SVS terrain layer) composes strictly below `Attitude`, in the
  band `Background` occupies. The PFD renders with `BackgroundMode::None`
  to cede that band. Horizon line, pitch ladder, roll scale, aircraft
  reference, tapes, and annunciations remain in the critical overlay and
  are byte-identical with the background present or absent.
- One frame is one encoded scene; frame generation and identity ride the
  transport (the WASM render generation), not the encoding. Each layer
  is owned by exactly one producer per frame — the duplicate rule makes
  split ownership structurally impossible.
- The layer validator accepts any legal ascending subset because different
  panel types use different bands. Before visible commit, the panel host
  must additionally require the critical-layer mask for the selected panel.

Backends that predate the layer markers skip them as unknown opcodes and
paint in encoded order. They still execute the ordinary Save/Restore
envelope, preserving state isolation and z-order, but they do not enforce
layer structure or resource budgets and are therefore not conforming
high-assurance consumers.

## Consequences

- One frame's rendering does one alloc-free pass over `no_std` code; browser,
  native, and embedded builds share every line above the backend seam.
- The scene IR version byte and the skip-unknown-opcode rule are the
  expensive-to-retrofit parts, so they ship in the first encoding.
- Ingest rate is decoupled from frame rate by construction: telemetry writers
  update state; backends render on their own cadence (pyG5's
  repaint-per-packet is the named anti-pattern).
- egui and Slint remain available as *hosts* for future native shells, but
  the instrument core cannot depend on them: egui's paint layer assumes
  std+alloc, Slint imposes its own UI description language and runtime, and
  both would put a third party's release cadence inside our
  certification-shaped display path. Rejected likewise: a separate repo (no
  shared CI/transport/client, premature independence) and a JS-first
  implementation (display logic trapped in one platform, violating
  ADR-0002).
- A future multi-display/reversion model (survey lesson from PRIME/Fusion:
  role-agnostic panes, deterministic reversion) is out of scope but not
  foreclosed: panels are already pure functions of state, so "which panel
  where" is host configuration, not panel code.
