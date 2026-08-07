# ADR-0034: The instrument extraction boundary — frame contract, artifact ownership, and gate placement

- Status: Accepted
- Date: 2026-08-06
- Extracted repository: <https://github.com/luofang34/Indicate> (private;
  consumers pin it by exact rev in their workspace manifests)

## Context

[#262](https://github.com/luofang34/Pilotage/issues/262) is the
checklist for extracting the instrument crate family into its own
repository. The mechanical blockers are resolved (the closure reaches
nothing outside `crates/`, and the closure's `no_std` gate passes in
one standalone invocation); what remains are the boundary decisions
that must be written down before a `git mv` freezes them. This record
states them; the extraction froze them.

## Decision 1 — the design-frame contract

A panel is authored in the logical frame its descriptor declares
(`design_frame`, 480×360 for every shipped panel). The contract:

- Every backend clips at the design frame: ink outside it never
  reaches a pixel, on any backend.
- Inside the frame, coordinates are logical units; backends scale to
  their surface without reinterpreting geometry.
- Unclipped text whose nominal ink extends past the frame edge is a
  COUNTED admission warning, ratcheted per panel — growth is a
  deliberate decision, not drift. The standing count is real display
  debt (the PFD `GS`/`SET` readout boxes draw at the requested size
  with no fit shrink); fixing the paint moves frame hashes and is its
  own change, at which point the ratchet steps down and warnings at
  zero can become failures.

## Decision 2 — artifact ownership at the boundary

- The scene-conformance corpus lives with the IR that defines its
  vocabulary: `crates/pilotage-instrument-scene/corpus/`. The
  reference rasterizer reads it as a sibling crate; the browser suite
  reads into the crate tree and pins `corpusSha256`, so a corpus edit
  reddens every consumer — that pin is the cross-repo sync mechanism,
  and the first post-extraction corpus change must be watched through
  it (ready-when check 2).
- The REN-04 timing artifact lives with the crate whose tests consume
  it: `crates/pilotage-instrument-raster/evidence/`.
- `docs/instruments/evidence-graph.evg` and its artifacts travel WITH
  the panels at extraction: its locators point into the closure and
  into the instruments docs that travel with it, and a
  graph that cannot resolve its own sources is dead weight on
  whichever side lacks them. The LINK/CTRL graphs stay behind.
- The AIR-* requirement registry and `check-instrument-requirements.sh`
  travel with the panels for the same reason; the id namespace does
  not split.

## Decision 3 — gate placement

- The closure `no_std` gate (`ci.yml`, one invocation, closure crates
  only) is the gate the extracted repository runs unchanged. The
  conformance harness is deliberately outside it: a host-side tool
  that allocates.
- The REN-03 raster baselines travel with their descriptors (already
  the case); the raster crate runs the gate on whichever side the
  crates live. `states::typical` is owned by the registry crate and
  moves with it.
- The browser shell's instrument half is protocol-free by crate
  boundary (`pilotage-instruments-web-shell`), enforced by a CI step,
  so the shell code that stays behind depends on the extracted family
  only through its published contract.

## Decision 4 — pin advance

The ecosystem pins sibling repositories by rev (Aviate, Navigate
precedent in the root `Cargo.toml`). The extracted instrument
repository is pinned the same way, and the PILOT of a change that
moves the cross-shell scene digest advances the pin in the consuming
repositories as part of that change — the digest is the migration
invariant, so a pin advance is complete exactly when every consumer
reproduces the new digest.

## Consequences

The `git mv` becomes executable: nothing in the closure reaches out,
the standalone gate exists, and each artifact has one owner. Of the two
checks that cannot run until after extraction, the closure gate runs
green in the extracted repository's own CI; the corpus sync is
exercised across repositories by the first post-extraction pin advance
that carries a corpus edit ([#262](https://github.com/luofang34/Pilotage/issues/262)
ready-when check 2).
