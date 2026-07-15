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
| Representable coordinate magnitude | 32767 px | `COORD_LIMIT_PX` (Rust + JS) |
| Worst-case frame size | 67108864 bytes | `WORST_CASE_FRAME_BYTES` |
| Coverage samples per frame | 1100000 | `RenderWork::BUDGET` |
| Composites per frame | 900000 | `RenderWork::BUDGET` |

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
`pilotage-instrument-raster::render`), so its work is a pure function of scene
bytes and framebuffer dimensions. That work is now *counted*, not just bounded:
every `RenderReport` carries a `RenderWork` record with

- `coverage_samples` — pixel-center coverage evaluations across all primitives
  (each bounded region loop counts one evaluation per pixel it visits);
- `composites` — source-over composites actually applied.

Because the counters are deterministic and platform-independent, an engineering
work budget gates them in CI today, before any display hardware exists to time.
`RenderWork::BUDGET` (1.1M coverage samples, 900k composites per frame) is
pinned at 2x the worst measured panel fixture — the fully populated PFD demo
scene measures ~547k samples / ~434k composites on the 480x360 panel, about 3.2
samples per output pixel — so scenes can grow denser without churning the
constant while per-frame work stays bounded at ~6.4 samples per pixel. The gate
is `panel_fixtures_fit_within_work_budget` (with
`work_counters_are_deterministic_and_nonzero` pinning purity), and exceeding the
budget is a hard CI failure with instructions to investigate the scene or the
region loops before raising the constant.

The per-frame cost for a specific target is the counted work priced per cost
class. `RenderWork` counts polygon edge tests, stroke segment tests, disc
tests, arc angular extras (cap distances, `atan2f`, `fmodf` — their own class,
so an arc sample is never billed as a bare disc test), and composites;
`timing::TargetTimingModel` derives a **provisional cost envelope** — not a
WCET claim until per-operation cycles are measured on selected hardware — and
gates it against the frame deadline derived from the SIM display liveness
requirement (`PanelHealth` `livenessDeadlineMs` = 1000 ms:
`budget_envelope_fits_the_display_derived_deadline`). No display hardware is
selected yet — the USB CDC scan (`scripts/detect-target.sh`) detects a
connected target and attempts an identity handshake rather than asking — so
the shipped model is the named conservative bound recorded, with its
assumptions and derivation, in
`docs/instruments/evidence-artifacts/timing/target-timing.txt`; a drift guard
keeps that artifact equal to the shipped constants. A measured model must bind
the firmware/build identity, MCU, clock and memory configuration, compiler
flags, and raw output; only then does the envelope become a WCET and the
deadline a display refresh requirement. The browser watchdog (`PanelHealth`,
simulator-only) latches a stalled panel as `LIVENESS` past its deadline.

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
| Paint fail-safe | non-finite coordinate, out-of-range coordinate, over-vertex-budget shape, non-finite rotation, out-of-range translation, non-finite arc angle, out-of-range stroke width |

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
- `interpreterRejects` — where the browser interpreter's raw-argument guards
  must throw (`coordinate`, `angle`, or `vertex-count`), predicted by the
  reference generator with the same rule and evaluation order;
- `canvasMethods` — the sequence of Canvas draw calls the interpreter must issue,
  captured from a command-recording canvas and compared as opcode + quantized
  args (omitted when `interpreterRejects` is set: the interpreter never reaches
  those calls).

Canonicalization is Q8.8: `floor(v * 256)`, with `nan`/`inf`/`-inf` sentinels.
Comparisons are exact — never tolerance-fudged.

### Conformance taxonomy

Differences surface as typed conformance failures, never numeric tolerances:
`FramingMismatch`, `DecodeVerdictMismatch`, `DecodeClassMismatch`,
`CommandTraceDivergence{index}`, `CommandTraceLengthMismatch`,
`UnknownOpcodeCountMismatch`, `CanvasDivergence{index}`, `CanvasLengthMismatch`,
`InterpretThrew`, `GuardMissing`. Any divergence fails the browser test with the
case name and the differing index. `GuardMissing` is the fail-closed direction:
a case the golden marks `interpreterRejects` that the interpreter paints anyway
means a browser-side guard has been weakened or removed.

### Fail-closed geometry guards on both backends

Canvas2D itself has no fail-safe for non-finite or absurd geometry — it will
happily accept a NaN rectangle or a ten-million-pixel translation. The browser
interpreter therefore does not let such arguments reach Canvas: `interpretScene`
guards every float that would become Canvas geometry (finite, and
`|v| <= COORD_LIMIT_PX` = 32767 for coordinates, sizes, radii, and stroke
widths; finite for rotation and arc angles) and every path against the shared
vertex budget (`MAX_PATH_VERTICES` = 512, the rasterizer's
`MAX_POLYGON_VERTICES`). A violation throws; `renderPanel` converts the throw to
`PAINT_FAILED` and discards the back buffer, so nothing partial reaches the
visible frame — the same commit-or-spoil outcome as the rasterizer.

One asymmetry remains, and it is a *rule* difference, not a missing guard: the
software rasterizer range-checks coordinates in **device space after the
transform** (that is where Q8.8 quantization happens), while the interpreter
rejects at the **raw-argument level** before Canvas applies the transform. The
raw rule cannot be transform-exact without reimplementing the rasterizer's
transform pipeline. For every scene the project encoder can produce this makes
no difference — panel geometry never relies on a transform to bring an
out-of-range raw coordinate back into range — and the corpus generator predicts
`interpreterRejects` with exactly the raw rule, so the golden pins the
interpreter's actual behavior, not an aspiration. A scene that deliberately
paired out-of-range raw arguments with a compensating transform would be
rejected by the interpreter and painted by the rasterizer; the conformance claim
for the browser backend is accordingly scoped to encoder-producible scenes, and
the browser remains SIM / NOT FOR FLIGHT.

No unexpected divergence was found between the backends at the levels they can be
compared: framing, decode, unknown-opcode counting, guard placement, and the
interpreter's opcode-to-Canvas mapping all agree across the corpus.

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
