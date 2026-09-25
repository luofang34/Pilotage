# ADR-0049: Shared records and authority are ready for collaboration across organizations

- Status: Proposed
- Date: 2026-09-23

## Context

Collaboration is future work. The architecture must not block it. Future
work includes:

- An operator terminal controls vehicles of other units or other
  organizations, within the rights that the owner grants.
- Operators share and deliver missions, marks on the map, and moving
  targets, for example a suspect vehicle in a law-enforcement pursuit.
- Operators coordinate different vehicles, for example a request for a
  helicopter to take over a pursuit at a named road.

Several records already support this:

- Leases and fencing generations apply to any principal (ADR-0006).
- Several operators can hold scopes on one vehicle, one client can attach
  many vehicles, and a coordinator host can control members (ADR-0028).
- The coordination server gives identity, rendezvous, and entitlements, and
  never carries session data (ADR-0027).

One part blocks collaboration. `PrincipalId`, `VehicleId`, and `SessionId`
are session-local integers (`pilotage-protocol`). They cannot identify a
principal or a vehicle across sessions, units, or organizations. A shared
record that stores them loses its meaning when it leaves the session.

## Decision

This record does not design collaboration. It sets the rules that code
must follow, so that collaboration can be added without a redesign.

### 1. Shared records use global, issuer-qualified identities

A record that can leave one session, such as a mission, a mark, a target
track, an assignment, or a handoff, uses:

- a globally unique record identifier, generated without coordination
  (a time-ordered UUID),
- an issuer-qualified principal: the identity issuer and the subject at that
  issuer,
- the owning organization, or the individual when there is no organization.

Session-local integers stay on the real-time wire for compact frames. The
session maps them to global identities. A shared record never stores a
session-local integer.

### 2. Every shared record has an origin, a revision, and releasability

Each shared record carries:

- `origin`: the principal and organization that made it,
- `revision`: an append-only revision number and the digest of the previous
  revision,
- `validity`: the time interval in which it applies,
- `releasability`: the organizations or roles that can receive it.

The owner filters a record before it leaves its organization. A receiver
gets only what the owner releases. For example, a mission has an aircraft
from one organization and a boat from a second organization. The owner
releases the search site to the boat. The owner does not release the track
of the aircraft to the boat.

### 3. Marks and moving targets are domain records, not map drawings

- A mark is a record of an annotation domain. The map renders it as a layer.
- A moving target is a track of the Surveillance domain (ADR-0036). A report
  from a person is an observation with a source and a confidence, fused like
  any other track source. The map renders the track.
- ADR-0036 defines the Surveillance domain for traffic. This rule widens the
  Surveillance domain to non-cooperative tracks, such as a ground vehicle
  that does not report its position.

The renderer never owns shared state. The same record reaches the map, the
agent context (ADR-0047), and other organizations.

### 4. A handoff is a mission revision and an authority handover

"Take over the pursuit at the named road" is:

1. a mission revision that adds an assignment for the receiving vehicle,
   with the target track, the transfer point, and the time,
2. an acceptance by the receiving owner,
3. an authority handover of the task scope under ADR-0010, with a new
   fencing generation.

No new primitive is added. The receiving vehicle can be in another session
and another organization, because the records use global identities.

### 5. Control across organizations uses the existing authority model

A vehicle owner grants a foreign principal a session capability for named
scopes (ADR-0006). The host verifies the foreign issuer against the trust
anchors that its policy lists. Leases and fencing then apply unchanged. The
coordination server can broker identity and entitlements. It never carries
session data (ADR-0027).

### 6. Records are exchanged, not synchronized in place

Shared records move as signed revisions between hosts, or through an
organization's mission service. A receiver can work offline and apply
revisions later. The revision chain detects conflicts. The owner resolves
them. Shared records do not require a central database.

### Performance

- A moving-target track updates on the telemetry class (ADR-0011), not as a
  document revision.
- Record revisions are small and immutable. A receiver requests only the
  revisions it does not have.

## Consequences

- New shared-record types must follow rules 1 and 2. A guardrail that checks
  new shared-record types for a global identifier and an origin is tracked as
  separate work.
- The session-local identifiers do not change. A mapping from session-local
  to global identities is required before the first shared record leaves a
  session.
- The annotation domain, the target-track class, cross-organization
  capabilities, and the handoff flow are tracked as deferred work.
