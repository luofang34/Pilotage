# Architecture Decision Records

One file per decision, numbered in acceptance order. See
[ADR-0001](0001-record-architecture-decisions.md) for the format and lifecycle rules.

## Index

| ADR | Decision | Status |
|---|---|---|
| [0001](0001-record-architecture-decisions.md) | Record architecture decisions as versioned files in this repository | Accepted |
| [0002](0002-cargo-workspace-portable-sans-io-core.md) | One Cargo workspace with a portable sans-IO core | Accepted |
| [0003](0003-separate-responsibility-planes.md) | Separate identity, authority, real-time data, media, and host planes | Accepted |
| [0004](0004-host-oriented-topology.md) | Host-oriented topology spanning hosted, peer-hosted, and real-vehicle sessions | Accepted |
| [0005](0005-webtransport-primary-transport.md) | WebTransport as the primary real-time transport, including media | Accepted |
| [0006](0006-capability-auth-scoped-leases-fencing.md) | Capability-based authorization with scoped leases and fencing generations | Accepted |
| [0007](0007-canonical-input-model-device-profiles.md) | Canonical input model and versioned device-profile registry | Accepted |
| [0008](0008-engine-independent-adapter-boundary.md) | Engine-independent adapter boundary; Gazebo first, reference adapter always | Accepted |
| [0009](0009-time-model-and-latency-budget.md) | Explicit time model, end-to-end latency budget, stale-input rejection | Accepted |
| [0010](0010-authority-state-machines.md) | Handover, override, and link loss as explicit state machines | Accepted |
| [0011](0011-message-classes-and-channel-semantics.md) | Separate control, telemetry, authority-event, and bulk message classes | Accepted |
| [0012](0012-structured-session-events.md) | Structured session events for observability, recording, and replay | Accepted |
| [0013](0013-interactive-and-accelerated-sessions.md) | Single-vehicle operation and horizontally scalable accelerated training | Accepted |
| [0014](0014-protobuf-wire-schema.md) | Protobuf as the wire-schema source of truth | Accepted |
| [0015](0015-workspace-quality-gates.md) | Workspace-enforced quality gates | Accepted |
| [0016](0016-codec-pluggable-media-plane.md) | The media plane is codec-pluggable; the control core never sees the codec | Accepted |
| [0017](0017-instrument-display-runtime.md) | Instrument display runtime as a no_std sans-IO crate family emitting a versioned scene-command IR | Accepted |
| [0018](0018-avionics-telemetry-and-aviate-adapter.md) | Avionics state rides telemetry additively; Aviate joins through a MAVLink adapter | Accepted |
| [0019](0019-pluggable-vehicle-link-shm-first.md) | Vehicle links are pluggable below the adapter; co-located SITL binds shared memory | Accepted |
| [0020](0020-video-capture-identity-and-clock-mapping.md) | Video frames carry a capture identity and an explicit clock mapping | Accepted |
| [0021](0021-simulator-camera-calibration-contract.md) | Versioned, hashed simulator camera/design-eye calibration | Accepted |
| [0022](0022-geospatial-projection-availability-contract.md) | Transport-independent geospatial, projection, and availability contract | Accepted |
| [0023](0023-vehicle-side-decomposition-fc-navigate-communicate.md) | Vehicle-side decomposition: flight control, Navigate, and Communicate behind typed contracts | Proposed |
| [0024](0024-navigation-authority-boundary.md) | Control-grade estimation stays with the flight controller; Navigate owns the global solution and guidance | Proposed |
| [0025](0025-client-optional-operation-automation-principals.md) | Client-optional operation: mission execution and agents are automation-class principals | Proposed |
| [0026](0026-host-capability-profiles.md) | Client posture follows discovered host capability: full-authority, data-gateway, or embedded | Superseded by ADR-0037 |
| [0027](0027-optional-coordination-server.md) | Optional coordination server: identity, rendezvous, and entitlements — never the session data plane | Proposed |
| [0028](0028-multi-vehicle-and-swarm-coordinator-hosts.md) | Multi-vehicle operation scales from roster attach to swarm aggregation via coordinator hosts | Proposed |
| [0029](0029-panel-layout-look-plugins.md) | Panels, layout, and look are data-driven plugins over the scene and state contracts | Accepted |
| [0030](0030-communicate-navdata-provisioning.md) | The host consumes aeronautical navdata through Communicate's cycle-dated snapshot surface | Proposed |
| [0031](0031-nav-guidance-telemetry-display.md) | Navigation guidance rides telemetry as its own stamped role; deviation scaling is display policy | Proposed |
| [0032](0032-ipad-native-client-shared-cores.md) | The Apple instrument composition boundary — Indicate owns the contract, the shells stay thin | Accepted |
| [0033](0033-panel-registry-config-and-digest.md) | Compose panel registries as data and use one scene digest across shells | Accepted |
| [0034](0034-extraction-boundary.md) | Keep instrument artifacts, gates, and pinned consumers at explicit repository boundaries | Accepted |
| [0035](0035-source-neutral-situational-services.md) | Keep situational services source-neutral and compose them through Pilotage | Superseded by ADR-0036 |
| [0036](0036-situational-domain-ownership.md) | Separate situational state by lifecycle | Accepted |
| [0037](0037-modular-operator-client-composition.md) | Compose operator clients from shared function modules | Accepted |
| [0038](0038-operator-surface-model.md) | One surface model for operator screens | Proposed |

## Provenance

These records supersede the pre-repository draft *Pilotage Architecture Decision
Records v0.3* (2026-07-05). Mapping from draft sections:

| Draft v0.3 | Successor | Notable changes |
|---|---|---|
| ADR-001 planes | ADR-0003 | Planes clarified as contract boundaries, not process boundaries; v1 ships two deployables |
| ADR-002 portable core | ADR-0002 | Core made explicitly sans-IO; crate list slimmed to a seed set that grows on demand |
| ADR-003 topology | ADR-0004 | Unchanged in substance |
| ADR-004 WebRTC | ADR-0005 | Replaced in redesign: WebTransport became Baseline across engines (Safari 26.4, 2026-03), so WebRTC is dropped from v1; media rides WebTransport + WebCodecs |
| ADR-005 capabilities/leases | ADR-0006 | Open question resolved: authority engine is host-embedded in v1 |
| ADR-006 input model | ADR-0007 | Unchanged in substance |
| ADR-007 adapter boundary | ADR-0008 | Reference headless adapter promoted to a v1 conformance deliverable |
| ADR-008 latency | ADR-0009 | Merged with the three-clock time model from draft ADR-012 |
| ADR-009 handover | ADR-0010 | Open question resolved: transfer commits at ACCEPT; third call is confirmation, not a gate |
| ADR-010 message classes | ADR-0011 | Unchanged in substance |
| ADR-011 events | ADR-0012 | Backpressure and drop accounting made explicit |
| ADR-012 training scale | ADR-0013 | Time model moved to ADR-0009 |
| — | ADR-0014 | New decision: wire format and schema evolution |
| — | ADR-0015 | New decision: repo-enforced lint, size, and CI gates |
