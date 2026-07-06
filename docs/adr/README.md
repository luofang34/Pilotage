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
| [0016](0016-codec-pluggable-media-plane.md) | Codec-pluggable media plane; the control core never sees the codec | Accepted |

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
