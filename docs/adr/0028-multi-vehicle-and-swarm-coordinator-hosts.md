# ADR-0028: Multi-vehicle operation scales from roster attach to swarm aggregation via coordinator hosts

- Status: Proposed
- Date: 2026-07-29

## Context

Clients and hosts are not one-to-one. One vehicle may be crewed by several
operators — one flying, one on a refueling or payload scope, one monitoring
sensors. One operator may command many vehicles, up to addressing a swarm as
a whole rather than any specific vehicle. And hosts must eventually
collaborate with no terminal attached at all.

The wire and the state machines are already vehicle-plural: authority is
keyed per (vehicle, scope) with independent generations
([ADR-0006](0006-capability-auth-scoped-leases-fencing.md)), every control
frame, lease, authority event, and telemetry sample carries a vehicle
identifier, one principal may hold scopes on many vehicles, and sessions may
contain multiple vehicles ([ADR-0013](0013-interactive-and-accelerated-sessions.md)).
What is vehicle-singular is deployment: each host process embeds one adapter
advertising one vehicle, and the client drives one vehicle.

## Decision

- **Multiple operators on one vehicle is the existing design**
  ([ADR-0006](0006-capability-auth-scoped-leases-fencing.md),
  [ADR-0010](0010-authority-state-machines.md),
  [ADR-0011](0011-message-classes-and-channel-semantics.md)). Completing it
  is mostly roadmap: the handover/override wire vocabulary and
  authenticated principals realize accepted designs. The one element this
  record adds is an **observer admission** — telemetry and media with no
  grantable scopes — for monitoring stations.
- **One client, many vehicles — roster attach.** A client MAY attach
  multiple sessions concurrently and hold scopes across them; the client
  keys all per-vehicle state, including media sources, by (session,
  vehicle) and never assumes global identifiers. A host MAY equally compose
  multiple vehicles behind one session through a composite adapter — a
  session already contains many independently addressed vehicles by design
  ([ADR-0013](0013-interactive-and-accelerated-sessions.md)). The adapter
  contract carries per-vehicle identity for capabilities, control,
  telemetry, and link-loss policy; extending the media-source surface with
  vehicle attribution is part of completing roster attach.
- **Swarm aggregation lives in a coordinator host.** A coordinator is a
  host role that advertises **aggregate scopes** (e.g. `swarm.motion`)
  through ordinary capability discovery. Holding an aggregate scope leases
  the coordinator's decomposition authority: the coordinator translates
  aggregate commands into per-vehicle commands and acts as an
  automation-class principal
  ([ADR-0025](0025-client-optional-operation-automation-principals.md)) of
  each member vehicle's host, over the ordinary client protocol. Fencing
  applies end to end: member hosts fence the coordinator exactly like any
  client, and displacing the coordinator on one member affects exactly that
  member.
- **Aggregate semantics are coordinator implementation, not wire
  vocabulary.** Formation geometry, role assignment, and member selection
  live in the coordinator; the wire stays per-vehicle frames plus aggregate
  scopes. A member vehicle needs no swarm concept at all.
- **Peer collaboration without a terminal reuses the same seam.**
  Host-to-host attachment over the client protocol is the reserved
  envelope for vehicles collaborating with no operator present; no separate
  peer protocol class is introduced. The concrete collaboration behaviors
  are deferred by design.

## Consequences

- Coordinator failure is safe by construction: each member scope engages
  its own link-loss policy ([ADR-0010](0010-authority-state-machines.md)),
  including *engage automation* for mission continuation
  ([ADR-0025](0025-client-optional-operation-automation-principals.md)).
- Who may lease aggregate scopes, and how aggregate authority interacts
  with a per-member holder (a swarm command racing a direct member lease),
  joins the [ADR-0010](0010-authority-state-machines.md) policy matrix.
- The roster client is a client-plane feature: vehicle list, per-vehicle
  authority display, and media keyed by session — no protocol change.
- A coordinator host is a normal host: registered, discovered, and
  admitted like any other ([ADR-0004](0004-host-oriented-topology.md),
  [ADR-0027](0027-optional-coordination-server.md)).

## Alternatives considered

- **Client-side fan-out as the swarm mechanism:** rejected as the
  destination — coordination dies with the client and offers no envelope
  for terminal-less collaboration. It remains the degenerate near-term
  case that roster attach provides for free.
- **Server-mediated fleet command:** rejected; it places a center in the
  command path, contradicting
  [ADR-0003](0003-separate-responsibility-planes.md) /
  [ADR-0004](0004-host-oriented-topology.md).
- **Swarm vocabulary on the wire (broadcast frames, group addresses):**
  rejected for now; it would push aggregation semantics into every host
  and client, and the coordinator model covers the scenarios with the
  vocabulary already deployed.
