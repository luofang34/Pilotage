# ADR-0023: Vehicle-side decomposition: flight control, Navigate, and Communicate behind typed contracts

- Status: Proposed
- Date: 2026-07-29

## Context

The session host is the companion layer two sibling systems already point at.
Aviate defines itself as a motion-control kernel — state estimation,
stabilization, actuation — excludes navigation, mission management,
networking, and UI by specification, and points full protocol support at a
companion computer. aerocontext (repository `v99n62`) assembles
provenance-tracked advisory aeronautical context (FSS briefings — against the
vendor's test service today — NOTAMs, weather, NASR/CIFP navdata) under a
normative advisory-only boundary, with command escalation deliberately gated
on an authority model it does not own. [ADR-0019](0019-pluggable-vehicle-link-shm-first.md) already
records the target topology in which the vehicle splits into flight-control,
navigation, and communication components; the navigation component does not
exist yet, and no record names the constellation.

The decomposition must serve: flying with no client attached (preloaded plan,
loss-of-communication procedures — [ADR-0025](0025-client-optional-operation-automation-principals.md));
live instrument, warning, and context data to clients when present; slow AI
agents as future commanders ([ADR-0025](0025-client-optional-operation-automation-principals.md));
and EFB-style preflight with no vehicle at all
([ADR-0026](0026-host-capability-profiles.md)).

## Decision

- The Pilotage **system** comprises three deployment roles: the vehicle-side
  **host**, one or more **operator clients**, and an optional **coordination
  server** ([ADR-0027](0027-optional-coordination-server.md)). This record
  decides the vehicle side.
- The vehicle side is three separately governed components composed by the
  Pilotage host:
  1. **Flight control** — the FC (Aviate first; PX4 through its adapter):
     control-grade estimation, stabilization, actuation. It joins through the
     `VehicleAdapter` boundary ([ADR-0008](0008-engine-independent-adapter-boundary.md))
     over its declared links ([ADR-0018](0018-avionics-telemetry-and-aviate-adapter.md),
     [ADR-0019](0019-pluggable-vehicle-link-shm-first.md)). The FC never
     depends on the other components.
  2. **Navigate** — a new sibling repository owning global navigation:
     modular filter-based multi-sensor fusion (GNSS, celestial, visual, and
     others; operable with any subset including a single source), integrity
     assessment, flight-plan management and execution, guidance, and terrain
     awareness (EGPWS-class). The authority split with the FC is
     [ADR-0024](0024-navigation-authority-boundary.md).
  3. **Communicate** — the aerocontext repository evolved in place, playing
     the communication role: external aeronautical context (FSS briefings,
     NOTAMs, weather, NASR/CIFP navdata; FIS-B, ADS-B, and traffic as they
     land) with structural provenance and freshness. Its advisory-only
     boundary is preserved verbatim: Communicate products never gate,
     command, or actuate.
- The **host orchestrates**: the session/authority/media endpoint it already
  is, plus component supervision and lifecycle, and the contracts below. The
  host remains the unit of deployment
  ([ADR-0004](0004-host-oriented-topology.md)).
- **Contracts are typed boundaries, not process boundaries**
  ([ADR-0003](0003-separate-responsibility-planes.md) discipline). Each
  component contract MUST be defined so that in-process linking (library
  crates) and sidecar processes are both conforming deployments. First
  increments MAY link Navigate and Communicate cores in-process; a later
  split into processes MUST NOT change any contract. Navigate domain logic
  follows the sans-IO rule ([ADR-0002](0002-cargo-workspace-portable-sans-io-core.md));
  Communicate's cores already take time as data.
- **Two data disciplines, never blurred.** Measurements (FC and Navigate
  state) ride the telemetry plane under stamped, coherence-gated admission
  ([ADR-0018](0018-avionics-telemetry-and-aviate-adapter.md)). Advisory
  context (Communicate products) rides bulk/advisory message classes under
  provenance and freshness tracking. An advisory product MUST NOT be
  presented or consumed as a measurement, and a measurement MUST NOT be
  re-served as advisory context without its stamps.

```text
Operator clients ⇄ WebTransport ⇄ Pilotage host ─ adapter ─ FC (Aviate | PX4)
                                     ├─ Navigate    (fusion, FPL, guidance, EGPWS)
                                     └─ Communicate (advisory context, provenance)
```

## Consequences

- Repositories stay separately governed with their own licenses and release
  cadence; cross-repo consumption uses pinned dependencies (the
  `aviate-xil-contract` precedent) or a process surface.
- Navigate's guidance commands enter the same fenced control path as any
  operator ([ADR-0024](0024-navigation-authority-boundary.md),
  [ADR-0025](0025-client-optional-operation-automation-principals.md)) — no
  privileged side door to the adapter exists for on-vehicle components.
- Communicate's gated command seam (`ControlCommandProvider`) maps onto
  Pilotage authority: escalation beyond advisory happens only through leased
  scopes ([ADR-0025](0025-client-optional-operation-automation-principals.md)).
- The moving-map and EFB data path is defined: navigation solution
  (Navigate) + navdata snapshots (Communicate) + signed terrain packages
  (`pilotage-svs-db`).
- "Communicate" is a role name; aerocontext keeps its product identity, CI,
  and publishing.

## Alternatives considered

- **Fold navigation and data into the host binary as modules:** rejected;
  it erases the certification and fault boundaries both sibling systems
  already assert, and couples release cadence across three products.
- **A new Communicate repository lifting aerocontext crates:** rejected for
  now; it duplicates a maturing product's CI and publishing. Revisit if the
  vehicle role and the EFB/agent product diverge materially.
