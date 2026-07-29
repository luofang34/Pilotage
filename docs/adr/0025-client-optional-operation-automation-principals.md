# ADR-0025: Client-optional operation: mission execution and agents are automation-class principals

- Status: Proposed
- Date: 2026-07-29

## Context

The host must fly the vehicle with no client attached: a preloaded flight
plan or mission, and loss-of-communication procedures that regulation
requires, must execute headless. Today every control frame originates in a
client; nothing host-side can originate flight.

The authority machinery already anticipates non-human commanders:
[ADR-0006](0006-capability-auth-scoped-leases-fencing.md) gives every lease
an authority class, [ADR-0010](0010-authority-state-machines.md) names
automation-agent acquisition and release and engages link loss per scope
from a per-vehicle, adapter-published action menu that already includes
engaging an automation mode, and the authority engine implements an
`Automation` class.
Communicate ([ADR-0023](0023-vehicle-side-decomposition-fc-navigate-communicate.md))
already serves AI agents through an MCP surface whose command escalation is
deliberately gated on an authority model.

## Decision

- **The mission executor is a principal, not a privilege.** Navigate's
  flight-plan execution, supervised by the host, acquires control scopes
  through the same authority engine as any client — an automation-class
  principal with fenced generations, liveness watchdogs, and audit events.
  No component-to-adapter bypass exists.
- **Preloaded plans, missions, and procedures are host-resident
  configuration**, loadable before departure and replaceable in flight by an
  authorized principal. Client absence is normal operation, not a failure
  mode.
- **Loss of communication engages automation through link-loss policy.** The
  *engage automation mode* action already in the adapter-published menu
  ([ADR-0010](0010-authority-state-machines.md)) receives its concrete
  semantics: when a vehicle's selected policy is automation engagement, a
  scope losing its holder is handed to the mission executor, which flies the
  applicable configured procedure (loss-of-comm procedure, mission
  continuation, return-and-land class). Policy selection stays per vehicle
  and engagement and clearance stay per scope
  ([ADR-0008](0008-engine-independent-adapter-boundary.md),
  [ADR-0010](0010-authority-state-machines.md)). Which procedure applies is
  vehicle and deployment configuration, selected before flight.
- **Humans recover authority through the normal machinery.** Automation
  yields through handover per policy; emergency override displaces it
  exactly as it would a human holder, advancing the fencing generation.
- **Slow AI agents attach as agent-class principals.** Advisory access flows
  through Communicate's MCP surface and confers no authority. Command
  escalation happens only by holding leased scopes under the same fencing —
  never a parallel path. Agent pacing rides the existing message classes;
  watchdog and deadman machinery applies unchanged, so a stalled agent is
  fenced out like a silent operator.

## Consequences

- Headless operation is a first-class host mode; the client is an observer
  and commander that may come and go.
- UI MUST show automation as the effective holder of a scope exactly as it
  shows a human, extending the corresponding
  [ADR-0010](0010-authority-state-machines.md) consequence.
- The authority policy matrix (which classes may preempt automation, veto
  rules, renewed-verification requirements) remains the
  [ADR-0010](0010-authority-state-machines.md) open question, now including
  automation and agent rows.
- The regulatory content of procedures — what a loss-of-comm procedure
  requires in a given airspace — is configuration data with provenance, not
  platform code.
- Structured session events ([ADR-0012](0012-structured-session-events.md))
  record automation acquisition, enactment, and yield transitions for
  replay and audit.

## Alternatives considered

- **A privileged host-internal control path for autonomy:** rejected; it
  would bypass fencing, watchdogs, and audit — precisely the machinery that
  makes displacing a faulty commander safe.
- **Agents as a separate protocol surface with their own authority model:**
  rejected; two authority models over one actuator set cannot both be
  authoritative.
