# ADR-0010: Handover, override, and link loss as explicit state machines

- Status: Accepted
- Date: 2026-07-05

## Context

Normal handover should feel like the positive three-call exchange used in aviation
and maritime practice ("You have control." / "I have control." / "You have
control."), with both parties certain who is expected to act. This normal process
must coexist with emergency takeover, supervisory authority, disconnected operators,
and automation agents — without ambiguity about the exact instant authority changes.

## Decision

Authority is maintained per control scope by the authority engine (ADR-0006) as an
explicit state machine.

### States and transitions

```text
Unassigned -> Held(A)
Held(A)    -> Offered(A -> B) -> Held(B)          normal handover
Held(A)    -> Released        -> Unassigned
Held(A)    -> Revoked         -> Unassigned | Held(C)
Held(A)    -> EmergencyOverride(C) -> Held(C)
Held(A)    -> LinkDegraded(A) -> Held(A) | LinkLost(A) -> VehicleConfiguredFailover
```

### Commit point (resolves a draft open question)

Effective authority changes at **exactly one atomic point: the authority engine
committing the recipient's ACCEPT**, which reassigns the lease and advances the
fencing generation.

```text
A: OFFER_CONTROL(scope, B)            engine: Offered; A remains effective holder
B: ACCEPT_CONTROL(scope, expected_generation)
                                      engine: atomic commit -> Held(B), generation+1
B: I_HAVE_CONTROL(scope, new_gen)     confirmation, audited, surfaced in UI
A: YOU_HAVE_CONTROL(scope, new_gen)   confirmation, audited, surfaced in UI
```

The three-call phraseology is the UX and audit layer on top of the two-phase
offer/accept commit. The confirmations do not gate the transfer: after commit, A's
frames are already fenced out by generation. Missing confirmations raise a UI warning
and an audit event, not a rollback. Offers expire after a configurable TTL; expiry
returns the scope to `Held(A)` with an audit event.

### Emergency and higher-authority operations

Distinct policy-controlled operations exist for: voluntary handover; requested
takeover requiring current-holder approval; emergency override; supervisory or
instructor override; administrative revocation; automatic reassignment after link
loss; automation-agent acquisition or release.

Emergency override MUST: be explicitly authorized for actor and scope; advance the
generation atomically; invalidate the previous holder immediately; be idempotent;
carry an override reason and authority class; produce conspicuous UI and audit
events; and never depend on the displaced holder's acknowledgement.

### Link loss

Link-loss behavior is *selected* per vehicle instance (vehicle class MAY supply a
default). The adapter publishes its supported actions (neutralize, brake, hold
briefly, pause, engage automation mode); the platform prescribes no universal action.

Link-loss is **engaged, cleared, and enacted PER SCOPE**, not vehicle-wide. Losing
(or releasing) one scope's holder engages that scope's policy on its own — it MUST
NOT drive any other scope of the same vehicle to failover, so dropping
`vehicle.gimbal` never brakes `vehicle.motion`. Each scope recovers independently:
a scope's engaged policy clears only after a fresh fenced generation installs a new
holder AND that holder demonstrates the scope's neutral activation condition ON THAT
SCOPE. The recovering client resumes only on the host's per-scope `LinkLossCleared`
acknowledgement, which the host emits only after the adapter CONFIRMS it cleared
that scope's latch (a refused clear is retried, keeping the scope neutralized until
it takes). The per-vehicle selection is the menu each scope draws its one policy
from; the enactment boundary is [ADR-0008](0008-engine-independent-adapter-boundary.md#amendments).

## Consequences

- UI MUST show the *effective* holder of every scope and distinguish pending from
  completed transfer.
- Callouts SHOULD be generated for normal handover and MUST be generated for forced
  override.
- State-machine tests MUST cover duplicate, delayed, reordered, and contradictory
  acknowledgements, offer expiry racing accept, and override racing normal transfer.
  The sans-IO engine (ADR-0002) makes these plain table-driven unit tests.

## Open questions (policy, not mechanism)

- Which authority classes may force-override which scopes.
- Whether the current holder can veto a requested (non-emergency) takeover.
- Which actions require renewed passkey verification.
- Whether physical-control acknowledgement is required or UI acknowledgement
  suffices, per deployment class.
