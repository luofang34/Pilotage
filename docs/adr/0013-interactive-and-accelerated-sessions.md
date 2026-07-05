# ADR-0013: Single-vehicle operation and horizontally scalable accelerated training

- Status: Accepted
- Date: 2026-07-05

## Context

Pilotage must serve an interactive operator controlling one vehicle *and* large
in-silico workloads with many vehicles, environments, policies, and simulation
instances. ADAS and autonomy development needs faster-than-wall-clock simulation,
deterministic seeds, batch stepping, headless execution, and distributed workers.
Interactive video streaming and human control must not be imposed on every training
instance — and real vehicles must not be forced into simulator semantics.

## Decision

- A session MAY contain one vehicle, multiple coordinated vehicles, or a simulation
  shard containing many independently addressed vehicles. Vehicle identity, control
  scopes, telemetry streams, authority, and lifecycle are namespaced by session and
  vehicle identifiers.
- The adapter contract supports both real-time streaming and explicit stepped
  execution (`step()` in ADR-0008); capability negotiation states whether an adapter
  is real-time, accelerated, deterministic, step-driven, render-capable, or
  physically embodied.
- Headless operation, deterministic reset, seeded scenarios, snapshot/restore,
  batched observations, batched actions, and accelerated execution are first-class
  adapter capabilities.
- Video and human-facing media are optional capabilities; training workers operate
  without rendering or media streaming.
- Canonical action and observation models MAY have efficient batch encodings but
  MUST preserve the semantics of the interactive control and telemetry models — one
  vocabulary, two densities.
- A fleet orchestrator MAY schedule workers, scenarios, policies, and datasets, but
  is outside the session data-plane correctness boundary (ADR-0004).
- Real vehicles advertise unsupported capabilities (stepping, reset, snapshot,
  deterministic replay) explicitly.

## Consequences

- The three-domain time model (ADR-0009) is what lets simulation time run
  decoupled from wall clock.
- Interactive operator services and training orchestration may become separate
  binaries while sharing protocol and model crates.
- Performance testing covers both one low-latency interactive session and many
  headless accelerated instances.

## Alternatives considered

- **Training as a separate product:** rejected; it would duplicate vehicle schemas,
  actions, telemetry, scenario definitions, and replay infrastructure.
- **Forcing all simulation through a video/game-session abstraction:** rejected;
  rendering and wall-clock pacing would make large-scale training inefficient.
