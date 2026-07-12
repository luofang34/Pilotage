# Renderer verification: conformance, budgets, and fault injection

This document is the verification wall for the instrument rendering backends. It
defines what each backend must prove before a frame reaches a display, records
the hard resource budgets, states the target-independent execution-time
measurement method, and specifies the shared conformance corpus that cross-checks
the browser Canvas interpreter against the deterministic reference rasterizer.

The Canvas/browser backend is **SIM / NOT FOR FLIGHT**. Its verification is
engineering conformance for the simulator; it makes no certification claim.

## Backends and their gates

| Backend | Where | Determinism gate |
|---|---|---|
| Reference software rasterizer | `pilotage-instrument-raster::render` | Bit-exact: pinned SHA-256 frame hashes (`src/raster/tests/frame_hashes.rs`), reproducible via `libm` + IEEE-754 `f32`. |
| Browser Canvas2D | `clients/web/instruments.js` (`interpretScene`) | Semantic conformance + documented, limited tolerances. **Not** pixel-deterministic — Canvas2D anti-aliasing and font/geometry rounding are platform-owned. |
| wgpu / embedded framebuffer | future | Consume the identical scene IR; verified by the same corpus when they land. |

Both backends share one authoritative structural gate: the strong layer contract
`pilotage-instrument-scene::validate_layers`. In the browser it runs inside the
wasm renderer (`clients/web-instruments`) on the scene the wasm itself produced,
before any bytes reach `interpretScene`; `interpretScene` re-checks framing
(`validateSceneStructure`) as defence in depth.

## Resource budgets

Pinned from the crate constants (`pilotage-instrument-scene::layer`,
`pilotage-instrument-raster`), and mirrored in the golden manifest's `budgets`
block so both sides gate against the same numbers. Exceeding any of these fails
the whole frame before anything becomes visible.

| Resource | Bound | Constant |
|---|---:|---|
| Defined layers | 6 | `LAYER_COUNT` |
| Commands per layer (incl. isolation save/restore) | 4096 | `MAX_LAYER_COMMANDS` |
| Graphics-state depth (incl. isolation save) | 32 | `MAX_STACK_DEPTH` |
| Encoded bytes per scene | 65536 | `MAX_SCENE_BYTES` |
| UTF-8 bytes per text run | 250 | `MAX_TEXT_BYTES` |
| Framebuffer dimension (per axis) | 4096 px | `MAX_DIMENSION` |
| Vertices per polyline/polygon command | 512 | `MAX_POLYGON_VERTICES` |
| Worst-case frame size | 67108864 bytes | `WORST_CASE_FRAME_BYTES` |

The corpus exercises the scene-byte, per-layer-command, and stack-depth budgets
at and one past their limits (`resource_budgets_flip_verdict_at_their_limits`),
and the framebuffer-geometry budgets through the rasterizer's typed errors
(`framebuffer_geometry_budgets_are_enforced`). Command count, stack depth, buffer
usage, and scene bytes gate in `validate_layers`; framebuffer memory gates in
`render` before the buffer is touched. Assurance-derived limits from the AIR-02
hazard analysis will replace or tighten these engineering placeholders; the
numbers here are conservative with the rationale recorded alongside each
constant.

## Target-independent execution-time measurement

The reference rasterizer is straight-line over the scene and framebuffer with no
I/O and only documented-bounded loops (see the crate docs on
`pilotage-instrument-raster::render`), so a target-independent worst-case
execution time is a sum of bounded step counts:

- command dispatches: at most `MAX_LAYER_COMMANDS` times the layer count;
- per-pixel coverage tests: at most framebuffer pixels times a shape's edges.

A step-counting harness can wrap the coverage predicate and command dispatch
without changing output. The WCET for a specific target is that step-count sum
multiplied by the selected target's per-step cycle bound; measured cycle costs
and the final WCET wait for hardware selection and are out of scope here. Frame
time is one of the gated budgets: a target that cannot meet its frame deadline
fails, and the browser watchdog (`PanelHealth`, simulator-only) latches a stalled
panel as `LIVENESS` past its deadline.

## The conformance corpus

`clients/web/scene-conformance-corpus.json` is a reviewed, versioned golden. The
reference rasterizer authors it (`pilotage-instrument-raster`, module
`src/raster/tests/conformance`); the browser test
`clients/web/scene-conformance.test.mjs` replays it. Both sides pin to it, and a
SHA-256 over the concatenated case bytes (`corpusSha256`) guards accidental
drift on both sides.

### Coverage map

| Category | Cases |
|---|---|
| Normal displays / every opcode | empty-background canonical, every drawing opcode, multi-layer PFD, transforms, clip, guidance, background imagery |
| Validity/failure imagery | failure-display band, annunciation flag |
| Extreme attitude / coordinates | large roll & translate, large in-range coordinate |
| Text / glyphs | covered digit, uncovered character |
| Version policy | unknown opcode counted (inside and outside a layer) |
| Malformed / truncated | bad version, truncated tail, malformed known payload, unknown layer id, full truncation sweep |
| Layer structure | duplicate, out-of-order, nested, end-without-begin, end-mismatch, unclosed, command-outside-layer, unisolated state, unbalanced state |
| Resource exhaustion | scene bytes, per-layer commands, and stack depth — each at and one past the limit |
| Paint fail-safe | non-finite coordinate, out-of-range coordinate, over-vertex-budget shape |

Budget-boundary streams that would be megabytes of hex are carried as a compact
`generator` descriptor (`fill_bytes`, `repeat_unknown`, `nest_saves`) that both
backends reconstruct byte-identically; the corpus hash covers the reconstructed
bytes, so a generator divergence surfaces immediately.

### What each side checks (capability asymmetry)

Given arbitrary scene bytes, the browser backend can run framing
(`validateSceneStructure`), a wire decoder, and the interpreter
(`interpretScene`); it cannot run the strong layer gate, which lives in wasm and
only runs on wasm-generated scenes. So the golden pins, per case:

- `framingValid` — the browser framing gate must return exactly this;
- `decode` + `commandTrace` — the browser wire decoder must agree (opcode +
  Q8.8-quantized args);
- `gate` (verdict, typed error class, unknown-opcode count, layers-present,
  per-layer command counts) — the reference layer-gate result, cross-checked on
  the Rust side and used as the reference verdict;
- `render` (ok + typed error class) — the reference rasterizer outcome;
- `canvasMethods` — the sequence of Canvas draw calls the interpreter must issue,
  captured from a command-recording canvas and compared as opcode + quantized
  args.

Canonicalization is Q8.8: `floor(v * 256)`, with `nan`/`inf`/`-inf` sentinels.
Comparisons are exact — never tolerance-fudged.

### Conformance taxonomy

Differences surface as typed conformance failures, never numeric tolerances:
`FramingMismatch`, `DecodeVerdictMismatch`, `DecodeClassMismatch`,
`CommandTraceDivergence{index}`, `CommandTraceLengthMismatch`,
`UnknownOpcodeCountMismatch`, `CanvasDivergence{index}`, `CanvasLengthMismatch`,
`InterpretThrew`. Any divergence fails the browser test with the case name and
the differing index.

### Documented intentional divergences

Where the two backends legitimately differ, the golden records both outcomes and
each side checks its own — these are not bugs:

- **Non-finite and out-of-range coordinates**, and **shapes past the vertex
  budget**: the software rasterizer spoils the frame (typed `NonFinite`,
  `CoordinateOutOfRange`, `TooManyVertices`); Canvas2D has no equivalent
  fail-safe and paints them. The browser is SIM / NOT FOR FLIGHT; a flight-class
  backend must add these guards.

No unexpected divergence was found between the backends at the levels they can be
compared: framing, decode, unknown-opcode counting, and the interpreter's
opcode-to-Canvas mapping all agree across the corpus.

### Golden review policy

The golden is versioned (`schemaVersion`, `corpusVersion`) and carries `review`
metadata (reason, approver). CI only ever *compares*; it never rewrites the file.
A change is a reviewed action:

1. Edit the corpus (`.../conformance/corpus`).
2. Bump `CORPUS_VERSION` and update `REVIEW_REASON` in
   `.../conformance/manifest.rs`.
3. Regenerate: `REGEN_CONFORMANCE_CORPUS=1 cargo test -p pilotage-instrument-raster regenerate_golden_when_requested`.
4. Review the diff and commit.

`golden_matches_reference` fails CI if the checked-in file is not exactly the
reference's serialization, so the golden can never silently drift from the
reference.

## Fault injection

| Fault | Reference (Rust) | Browser (JS) |
|---|---|---|
| Malformed scene (bad version, truncation sweep, malformed payload, bad layer id) | `validate_layers`/`SceneCmds` typed error; `render` spoils | `validateSceneStructure` rejects framing; `renderPanel` → `SCENE_FRAMING` |
| Resource exhaustion (bytes/commands/stack at & past limits) | verdict flips exactly at the limit | framing valid both sides; strong gate enforced in wasm |
| Backend failure | framebuffer-limit typed errors leave buffer untouched or spoil | a throwing Canvas op → `PAINT_FAILED`, contained to the back buffer |
| Unsupported opcode | counted skip, `unknown_opcodes` reported | counted skip, `interpretScene` returns the same count |
| Glyph corruption / uncovered text | `render` → `Glyph` error, no substitution | `interpretScene` throws, no font fallback (load-time hash checks already covered) |
| Liveness / recovery | n/a (stateless) | `PanelHealth` latches on any fault, recovers only after sustained validated frames; `LIVENESS` on a stalled generation |

Old imagery is covered on every backend error or timeout: the reference
rasterizer spoils the whole frame (opaque black + red cross) so no stale frame
survives; the browser pipeline is transactional (validated frame to an offscreen
buffer before the visible blit) and covers failures with the backend-owned
failure page and an independent DOM fault surface.

## CI integration

The reference-side tests run under the existing `cargo test -p
pilotage-instrument-raster` step (they include the drift guard, coverage, budget,
and fail-safe tests). The browser-side conformance test is a new step alongside
the other `clients/web` node tests:

```
- name: renderer backend conformance corpus
  run: node clients/web/scene-conformance.test.mjs
```
