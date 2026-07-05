# ADR-0006: Capability-based authorization with scoped leases and fencing generations

- Status: Accepted
- Date: 2026-07-05

## Context

A vehicle may have multiple independently controlled subsystems: one user may hold
the **vehicle helm** while another holds the **camera helm** or **payload helm**.
Passkeys are right for login and sensitive re-authorization but cannot sign every
real-time input frame. Revocation and handover must invalidate a stale controller
immediately, even while its transport connection remains open.

## Decision

### Authentication and capabilities

- Authentication uses passkey/WebAuthn and produces a short-lived **session
  capability** binding principal → session → grantable scopes.
- The capability admits the principal to a session; it does not itself confer
  real-time control.

### Leases and fencing

- Real-time authority is one **lease per independently assignable control scope**.
- Each scope lease carries a monotonically increasing **fencing generation**.
- Every control frame identifies its target scope and lease generation; the host
  rejects (and counts) any frame whose generation is not *equal* to the current
  generation for that scope.
- A principal MAY hold multiple scope leases; different principals MAY hold
  different scopes on the same vehicle.
- Generations advance on every handover, revocation, override, or reassignment.

Scopes are published by host capability discovery, not hard-coded globally
(e.g. `vehicle.motion`, `vehicle.camera`, `vehicle.payload`,
`vehicle.automation-mode`). Operational vocabulary (*helm*, *watch*, *handover*,
*relief*) is a UI concern; wire identifiers stay stable, neutral, and
machine-readable.

```rust
struct ControlLease {
    holder: PrincipalId,
    vehicle: VehicleId,
    scope: ControlScope,
    generation: u64,
    authority_class: AuthorityClass,
    priority: u16,
    valid_until: MonotonicDeadline,
}
```

### Authority engine placement (resolves a draft open question)

The authority engine is a **sans-IO library crate (`pilotage-authority`) embedded in
the session host** for v1. The identity plane decides *who may enter and what they
may request*; the host-local engine is authoritative for *who holds what right now*.

Rationale: the per-frame verifier needs session-local state regardless, and a
central authority service would add a network dependency in the path that changes
effective control — exactly where partitions are most dangerous. Because clients
observe authority only through ordered authority events, the same engine can later be
fronted by a central or organizational authority service for fleet-wide policy
without protocol change.

## Consequences

- The per-frame verifier operates on compact session-local state and MUST NOT perform
  external policy lookups.
- A camera controller can be replaced without disturbing the motion controller.
- UI MUST show the current holder of each active scope separately.
- Authorization policy MAY grant rights to request, approve, release, or forcibly
  override specific scopes; the policy matrix is an open product question tracked in
  the backlog, not blocking the engine design.
- Rejected-frame counts per scope/generation are mandatory observability signals
  (ADR-0012).
