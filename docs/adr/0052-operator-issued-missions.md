# ADR-0052: Operator-issued missions

- Status: Proposed
- Date: 2026-10-22

## Context

A mission reaches the session host in one way: the operator sets
`PILOTAGE_MISSION_*` variables before the host starts
([ADR-0025](0025-client-optional-operation-automation-principals.md)). The
mission principal builds one document from them
([ADR-0041](0041-one-mission-sequencing-core.md)), flies it, and ends. The
operator cannot issue a mission from the App, change it in flight, or issue
the next one without a host restart. The App shows the mission's state only
through the authority events and the guidance group; it has no mission
object of its own. The physical-vehicle lane
([ADR-0051](0051-a-physical-vehicle-flies-on-its-own-estimate.md)) made the
operator's decisions part of a mission's life: the launch, the handover, the
release, the landing. Those decisions are expressed today through leases,
not through the mission.

The goal is a mission the operator issues and changes from the App while
the vehicle flies, that the host can execute without an operator action at
each step when the vehicle and its adapter support it, and whose every
transition the operator can see and veto.

## Decision

### A mission is a session object the operator issues

A `MissionDocument` ([ADR-0041](0041-one-mission-sequencing-core.md))
travels on the wire as the operator's statement of intent. The App
composes it: an area drawn on the map, a route, or a single target, with
the height, the speed, the camera, and the return behaviour. The App
submits it on the reliable stream under the operator's principal. The host
answers with a receipt that names the document's digest, the plan it built,
the figures the operator checks (lines, waypoints, path length, estimated
duration and energy), and the admission result: admitted, or refused with
the reason. The environment variables remain a bootstrap convenience for
unattended hosts and tests; an issued mission supersedes them.

### A mission is dynamic through revisions, applied at safe points

An amendment is a new document revision carrying the digest it replaces.
The host does not splice the running plan. It admits the revision and
applies it at the next safe point: a captured waypoint, a line end, or a
hold. The engine reports which revision is active; the App shows both until
the switch. A revision that the vehicle cannot fly under the advertised
envelope is refused as a whole. Cancelling is a revision with a single
phase: return to the launch point and hold.

### Operator decisions are mission commands, not only leases

The operator's decisions become explicit mission commands on the reliable
stream: `Start`, `Hold`, `Resume`, `Return`, `Abort`. The lease machinery
([ADR-0010](0010-control-authority-and-handover.md)) keeps its role as the
safety fence: a human taking the motion scope still holds the mission
([ADR-0051](0051-a-physical-vehicle-flies-on-its-own-estimate.md)), and
`Resume` is refused while a human holds it. The commands add what leases
cannot express: starting a mission the operator just issued, asking it to
come home, and aborting it. Every command is answered with a receipt, and
every transition is an event the App records with the operator's identity.

### Autonomy is a declared level, gated by advertised capabilities

The document declares the autonomy level the operator asks for:

- `Assisted`: the host executes one phase per operator `Start`; the mission
  holds at every phase boundary.
- `Supervised`: the host executes the whole document; the operator can
  hold, resume, return or abort at any time. This is today's behaviour.
- `Autonomous`: the host executes the whole document and the recovery
  policy on its own, including return and landing, without an operator.

Each level requires capabilities the adapter advertises: velocity control
and an authorized heading for any flight; return and land actions, battery
state, landed state and a link-loss policy for `Autonomous`. The host
refuses a document whose level the vehicle does not qualify for, and the
App shows which level the connected vehicle qualifies for before the
operator composes.

### The host owns execution; the App owns intent and consent

The host executes the admitted document and reports. The App composes,
submits, amends, commands, and records consent: an operator who issues an
`Autonomous` mission confirms the recovery policy (return height, battery
reserve, lost-link behaviour) in the same action. The App keeps the mission
object across reconnects by its digest and resynchronizes from the host's
status. The browser, native and Apple clients share this through
`pilotage-client-session`; no client duplicates the planner.

### Recovery is part of the document

A document carries its recovery policy: the return height, the battery
reserve the planner budgets for the way home, the link-loss behaviour (hold
in place, return, or land) and the geofence. The planner refuses a mission
whose estimated energy exceeds the reserve. The executor applies the policy
without an operator when the level allows it, and otherwise holds and asks.

## Consequences

- New wire messages: mission submission, revision, command, receipt and
  status; a new scope of authority, the mission scope, held by the issuing
  operator, separate from the motion scope the executor leases.
- The environment-variable mission becomes one source of a document; the
  mission principal gains an idle state in which it waits for a document.
- The App gains a mission composer on the map (area, route, target), a
  mission panel (state, revision, figures, commands), and a consent step
  for autonomous recovery.
- Adapters declare return and land actions, battery and landed state, and
  a link-loss policy as capabilities; the DJI adapter advertises none of
  them for `Autonomous` until they are flight-verified.
- Sequencing: wire messages and receipts first, with the environment
  bootstrap kept; then revisions at safe points; then autonomy levels and
  recovery policy; the composer in the App alongside each step. Flight
  verification of each step precedes the next.
