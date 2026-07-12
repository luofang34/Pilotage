# ADR-0019: Vehicle links are pluggable below the adapter; co-located SITL binds shared memory

- Status: Accepted
- Date: 2026-07-09

## Context

Bringing Aviate SITL telemetry into the session host over its MAVLink
subset surfaced a class of problems that were never about frame format
(the parser interoperates byte-for-byte): MAVLink leaves **session
semantics** undefined, so every deployment reinvents peering. Concretely:
the FC unicasts telemetry to the last peer that commanded it
("follow the commander"), the GCS UDP port is a bind race between a
router and a direct consumer, and router registration is an implicit
side effect of heartbeats. A second telemetry consumer — exactly what a
display host is — steals or loses the stream.

Meanwhile the target topology (browser → WAN → ground host → RF link →
vehicle, where the vehicle splits into FC / navigation / communication
components) makes clear there is **no single protocol that should span
the chain**: each hop has different constraints. WebTransport + protobuf
is settled for browser↔host (ADR-0005/0014). The host↔vehicle hop is
the open one.

## Decision

- **The vehicle link is a pluggable layer below the `VehicleAdapter`
  boundary.** One adapter (per vehicle family) selects among link
  bindings at startup; the telemetry plane above it is identical for
  all of them. The Aviate adapter's bindings today: `shm` (co-located),
  `mavlink` (routed/remote, and the PX4-ecosystem compatibility path),
  selected by `PILOTAGE_AVIATE_LINK` with an `auto` default that prefers
  shared memory when present.
- **Co-located SITL binds shared memory.** Aviate's gz-sim plugin
  publishes a seq-numbered latest-state block
  (`AviateSharedState` in its standalone-C `shared_state.h`) into POSIX
  shm every physics tick. The adapter attaches **read-only** as a second
  consumer, double-reads the sequence counter to reject torn snapshots,
  converts ENU/FLU → NED/FRD with math mirrored from Aviate's own
  conversion functions, and ages the block out (withholds telemetry)
  when the counter stops advancing. The reader requires the exact 216-byte
  layout and records `(device, inode, size)` before mapping. Reopening the
  same frozen object cannot reset freshness; a different object plus a
  coherent first sample advances the attachment epoch. Sequence or simulation
  time rollback within one object is quarantined. No ports, no peering, no
  single-consumer contention; the Pilotage reader remains Rust and adds no
  intermediary bridge process.
- **The RF-link protocol decision is deferred**, deliberately: it
  belongs to the vehicle's communication component (which owns the radio
  and may speak MAVLink, CCSDS, or a native framing), and designing it
  before that component exists would be speculation. What this ADR fixes
  now is the *seam* those protocols plug into. The MAVLink binding
  remains for ecosystem interop and carries Aviate's estimator authorization;
  a native Aviate link (subscribe/fan-out semantics, source incarnation,
  authenticated framing) still gets its own RFC with the Aviate side.
- **Unsafe containment.** Mapping shared memory requires `shm_open`/
  `mmap`. The adapter crate drops the workspace-level
  `unsafe_code = "forbid"` to `deny` with per-site `allow` and SAFETY
  justifications, confined to the shm module — the same containment
  pattern as the WASM export shim. The block is mapped `PROT_READ`, so
  the adapter cannot corrupt simulator or FC state by construction.

## Consequences

- The instrument runtime and everything above the adapter never learns
  which link produced a sample — the thin-display/thick-display axis
  from the ADR-0017 survey stays open all the way down.
- Shared memory reads simulator ground truth, so the adapter explicitly marks
  each atomic block Good with all represented signals valid. This authorization
  is simulator-only. The MAVLink binding instead consumes the FC estimator's
  lossless per-signal status and exact acquisition timestamp under ADR-0018's
  fail-closed join rules.
- Latency for co-located SITL drops to a memory read (the block is
  written at the 1 kHz physics rate), and the port-ownership races
  disappear — which, more than raw latency, is what shared memory buys;
  the dominant end-to-end latency items remain video encode (ADR-0016)
  and the WAN hop.
- A frozen simulator ages into flagged instruments rather than replaying
  stale state as live (the same withholding discipline as the Gazebo
  adapter's dead-reader path).
- The 216-byte SHM contract has no magic, version, size, writer incarnation,
  or clock epoch fields. POSIX object identity is sufficient only for the
  simulator boundary. Any producer seeking operational credit must expose an
  additive, source-issued incarnation and explicit clock epoch rather than
  infer either from shared-memory metadata.
