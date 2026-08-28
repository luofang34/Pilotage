# ADR-0039: The fleet model — sets of sessions, cells of authority

- Status: Proposed
- Scope: design only; implementation deferred
- Date: 2026-08-17
- Depends on: [ADR-0037](0037-modular-operator-client-composition.md)

## Context

Three operating shapes are coming:

1. One operator commands a swarm. Each vehicle carries its own host.
2. Several operators command one vehicle, split by function. The lease
   and scope machinery already serves this shape today.
3. Several operators command one swarm: an M-by-N matrix of operators,
   vehicles, and functions.

The question is what model carries all three without a redesign.
Authentication and authorization policy come later; the shapes must be
right first.

## Decision (proposed)

### Rule one: authority is never federated

Each vehicle's host stays the single arbiter of that vehicle's scopes:
admission, leases, generations, silence watchdogs, typed refusals. No
layer above the host grants or revokes anything. A cross-host
"swarm lease" would put distributed consensus inside a safety path;
this model forbids it permanently.

The unit of authority is the **cell**: one (vehicle, scope) pair under
one host, held by at most one principal. Today's wire already enforces
the cell. Every fleet feature composes cells; none merges them.

### Rule two: a fleet is a client-side set of sessions

The client generalizes from "the session" to a **session set**. One
sans-IO engine instance per host, exactly as today — the multi-lane
engine repeats one level up, unchanged. A fleet client holds N
admissions, N catalogs, N lease tables; a single-vehicle client is the
degenerate case with N = 1.

A swarm command is N independent fenced commands. Partial outcome is a
first-class, visible state — never papered over with pretended
atomicity. The arm order telegraph already has the right grammar: one
lever can order a SET, and each vehicle answers through its own report.
A formation telegraph is a bank of lamps under one lever; the aggregate
phase is the worst member's, and a refusing member is named, not
averaged away.

The operator's stick binds to a **controlled set**: the selected subset
of cells the demand fans out to. Selection is a client concern;
enactment stays per-cell and per-host.

### Rule three: crew plans assign, hosts decide

Coordination between operators is a **crew plan**: a document that maps
operator to cells. The plan travels over any convenient channel — a
SharePlay group session, a file, a QR code. It carries intent, never
authority. Each operator's client still requests each lease from each
host, and a conflict resolves exactly as today: holder present, then
the cooperative ask.

This keeps shape 2 (several operators, one vehicle) and shape 3 (a
crewed swarm) the same mechanism: a plan partitions the cell matrix,
and every cell still has one holder that one host enforces.

### Identity, in two layers

- A vehicle needs a fleet-stable identity. Today's `VehicleId` is
  host-local. The catalog grows a stable name (an operator-readable
  callsign and a durable id), so plans and interfaces never say
  "vehicle 1 at 192.168.1.224".
- An operator needs one identity across hosts. Today each admission
  mints an anonymous principal. Later, one operator key admits at every
  host and the per-host principal ids map back to it. The model only
  requires that the mapping EXISTS; the authentication scheme is a
  later record.

### What already fits

- The multi-lane client engine is the cell model inside one session.
- Cooperative handover generalizes per cell with no changes.
- The telegraph's order-versus-answer split was chosen for exactly this
  growth: levers scale to sets, lamps never lie.
- ADR-0037 composition: "fleet" is a client module with a shared core
  (session-set bookkeeping, fan-out, formation telegraph) and thin
  platform ports.
- ADR-0038 surfaces: a vehicle is a surface group; a fleet is a gallery
  of them; visionOS gives each vehicle its own window.

## Migration (all deferred)

1. Stable vehicle identity in the catalog.
2. A session-set facade over N client engines, behind the current
   single-session interface.
3. Controlled-set selection, demand fan-out, formation telegraph.
4. Crew plan document and role interface, transport-agnostic.
5. Operator identity and host-side authorization policy.

## Consequences

- No new trust anywhere: adding vehicles or operators adds cells, never
  a coordinator that can be wrong about who holds what.
- Partial failure is the normal, displayed case, which is the truthful
  posture for radio-linked fleets.
- The web client can adopt the same session-set core; nothing in the
  model is Apple-specific.
