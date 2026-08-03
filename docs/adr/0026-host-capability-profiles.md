# ADR-0026: Client posture follows discovered host capability: full-authority, data-gateway, or embedded

- Status: Proposed
- Date: 2026-07-29

## Context

The client must serve two postures with one codebase: a fully capable control
terminal beside a full host, and an EFB-style preflight and monitoring tool
beside a limited host — or no host at all, as on a tablet with no vehicle
attached. Limited hosts exist in practice as data gateways (FlightStream-class
receivers exposing AHRS, GNSS, ADS-B, FIS-B, and flight-plan exchange but no
actuators). Host capability discovery already exists: scopes are published,
not hard-coded ([ADR-0006](0006-capability-auth-scoped-leases-fencing.md)),
and adapters advertise what they cannot do
([ADR-0013](0013-interactive-and-accelerated-sessions.md)).

## Decision

- **A host advertises a capability profile** — the truthful union of what its
  components provide. Two advertised profiles anchor the hosted range:
  - **full-authority**: actuator scopes plus telemetry, media, and advisory
    context; the complete control terminal experience.
  - **data-gateway**: telemetry and advisory context with zero actuator
    scopes — e.g. a bridge to an AHRS/GNSS/ADS-B receiver, including
    flight-plan send/receive where the device supports it. A data-gateway
    host is an ordinary adapter family
    ([ADR-0008](0008-engine-independent-adapter-boundary.md),
    [ADR-0019](0019-pluggable-vehicle-link-shm-first.md)); no new protocol
    exists for it.
- **The third posture is host-absent: embedded.** No session host process
  runs at all — the client embeds Communicate cores in-process for
  preflight (aerocontext's device-clean crates make this a supported
  target). The same display contracts and data model apply; nothing is
  advertised because there is nothing to discover.
- **Clients adapt by discovery, not build variants.** EFB behavior is the
  client experience when no actuator scope is advertised or no host is
  present; the full terminal is the same client when authority scopes exist.
  A client MUST NOT hard-code the posture it expects.
- **Displays render honestly by capability**: absent capabilities produce
  the existing Missing-status discipline, never placeholders that imply a
  capability exists.
- **Preflight functions** — briefing on a live map, pack-for-flight data
  currency, on-device AI highlighting of briefings — come from Communicate
  cores in every profile, embedded or hosted. Features requiring vendor
  credentials or entitlements need the coordination server
  ([ADR-0027](0027-optional-coordination-server.md)) and degrade to absent
  without it.

## Consequences

- Session bootstrap handles the host-absent case as a normal path, not an
  error.
- The authority machinery is uniform: against a data-gateway profile there
  is simply nothing to lease; no client or host branch exists for "EFB
  mode".
- One client across web, iPadOS, and native rests on the portable cores
  ([ADR-0002](0002-cargo-workspace-portable-sans-io-core.md)) and the
  display plugin contracts
  ([ADR-0029](0029-panel-layout-look-plugins.md)).
- Profile truthfulness is a trust statement: a host MUST NOT advertise
  scopes its components cannot enact; capability advertisement joins the
  host trust model tracked in [ADR-0004](0004-host-oriented-topology.md).

## Alternatives considered

- **A separate EFB application family:** rejected; it forks the client
  architecture and the data model, and the EFB posture is a capability
  subset, not a different product shape.
- **Requiring a host process everywhere, embedded via localhost:** rejected;
  it forces a server runtime into a tablet app for pure preflight and adds a
  bootstrap path with no capability gain over in-process linking.
