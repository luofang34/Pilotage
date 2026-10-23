# ADR-0052: Operator-issued, node-based missions

- Status: Proposed
- Date: 2026-10-22

## Context

A mission reaches the session host in one way: the operator sets
`PILOTAGE_MISSION_*` variables before the host starts
([ADR-0025](0025-client-optional-operation-automation-principals.md)). The
mission principal builds one document from them
([ADR-0041](0041-one-mission-sequencing-core.md)), flies it, and ends. The
operator cannot issue a mission from the App, change it in flight, or issue
the next one without a host restart. The App shows the mission only through
the authority events and the guidance group; it has no mission object. An
area survey is one `FollowPlan` phase, and its lines live inside Navigate's
plan execution, which the mission core and the App cannot see. The agent
([ADR-0042](0042-agent-client-module-and-model-port.md)) has a directive
vocabulary and a lowering onto flight actions, and its executor cannot run
them, because the document does not change in flight. The physical-vehicle
lane ([ADR-0051](0051-a-physical-vehicle-flies-on-its-own-estimate.md)) made
the operator's decisions part of a mission's life, and expressed them
through leases only.

The goal is a mission that the operator issues and changes from the App
while the vehicle flies, node by node; that an agent can change through the
same path from a natural-language instruction; that the host can execute
without an operator action at each step when the vehicle qualifies; and
whose every transition the operator can see, record, and veto. The design
must keep each concern in one place, so the mission can grow for years
without a rewrite: what to do, where, how, who may, and what happened.

## Decision

### One mission lane per vehicle

The host keeps one mission lane per vehicle. The lane has a lifecycle:
`Idle`, `Admitted`, `Running`, `Held`, `Complete`, `Aborted`. A lane in
`Idle` waits for a document. The environment variables remain a bootstrap
source: they issue the first document into the lane when the host starts.
A document issued from a client supersedes them. The lane is the one place
that knows which document is active, which successor is admitted, and
which node is running.

### A mission is a list of nodes

A node is one bounded task with its own entry conditions, completion
conditions, abort conditions, and capabilities. A node is a `MissionPhase`
of [ADR-0041](0041-one-mission-sequencing-core.md); this decision gives the
phase its operational granularity. An area survey becomes one node per
line and one node for the way home. A route becomes one node per leg. The
node vocabulary is the flight-action vocabulary: climb to a height, fly a
segment, hold at a point for a time or for a signal, return to the launch
point, land. New kinds of work, such as an orbit or a point inspection,
add a node kind, not a mission kind.

The mission core owns the transition between nodes. The flight handler
binds one Navigate plan to one `FollowPlan` node and rebinds for the next
node; it does not bind one plan for the life of the mission. Navigate
starts each plan on a direct-to leg to its first waypoint, so the next
node begins where the vehicle is.

Until the survey document is node-granular, the host publishes the active
plan's remaining waypoints as the nodes of the one `FollowPlan` phase, so
the operator sees the same list either way.

### Documents are immutable; change is a successor

A `MissionDocument` is immutable and digest-identified
([ADR-0041](0041-one-mission-sequencing-core.md)). An amendment is a new
document, a successor, that carries the digest it supersedes and the node
it resumes at. The lane admits a successor while the active document runs
and applies it at the next safe point: the completion of the running node,
or at once while the lane is `Held`. The core gains one entry for this:
accept a successor, and switch at the next node boundary. The core never
splices a running document. A successor the vehicle cannot fly under the
advertised envelope is refused as a whole, with the reason. Cancelling is
a successor with one node: return to the launch point and hold.

Every document records who issued it: the operator, an agent, or a host
policy, with the client identity and the time. The chain of digests is the
history of the mission.

### Operator decisions are intents

The decisions become intents
([ADR-0047](0047-agents-read-one-context-port-and-act-through-intents.md)):
`Issue(document)`, `Revise(successor)`, `Start`, `Hold`, `Resume`,
`Skip(node)`, `Return`, `Abort`. One vocabulary serves the App's mission
panel, the gamepad profile, and the agent. The authority engine checks
each intent under a new scope, the mission scope, that the issuing
principal holds. The motion-scope lease keeps its role as the safety fence
([ADR-0010](0010-authority-state-machines.md)): a human who takes the
motion scope holds the mission
([ADR-0051](0051-a-physical-vehicle-flies-on-its-own-estimate.md)), and
`Resume` is refused while a human holds it. Every intent gets a receipt:
accepted and at which node it applies, or refused and why. Every
transition is an event with the principal that caused it.

### The mission is visible

The host publishes a mission status on the reliable stream when it
changes and at a slow steady rate: the active document digest and its
chain, the lane lifecycle, the autonomy level, the node list with each
node's state (done, active, pending, skipped), the admitted successor and
the node it waits for, the figures the operator checks (lines, path
length, estimated duration and energy), and the last receipt. The same
status is the mission entry of the agent context port
([ADR-0047](0047-agents-read-one-context-port-and-act-through-intents.md)),
so an agent and the App read one truth. A client keeps its mission object
across a reconnect by the digest and resynchronizes from the status.

### Autonomy is a declared level, gated by capabilities

The document declares the level the operator asks for:

- `Assisted`: the host executes one node per operator `Start`; each node's
  entry condition includes the operator's consent.
- `Supervised`: the host executes the whole document; the operator can
  hold, resume, skip, return, or abort at any time. This is today's
  behaviour.
- `Autonomous`: the host executes the whole document and the recovery
  policy on its own, including return and landing, without an operator.

Each level requires capabilities the adapter advertises: velocity control
and an authorized heading for any flight; return and land actions, battery
state, landed state, and a link-loss policy for `Autonomous`. The host
refuses a document whose level the vehicle does not qualify for, and the
status names the level the connected vehicle qualifies for before the
operator composes.

### Recovery is part of the document

A document carries its recovery policy: the return height, the battery
reserve the planner budgets for the way home, the link-loss behaviour
(hold, return, or land), and the geofence. The planner refuses a mission
whose estimated energy exceeds the reserve. The executor applies the
policy without an operator when the level allows it, and otherwise holds
and asks. A recovery the host starts on its own is a successor it issues
as a host policy, so it appears in the chain like every other change.

### Natural language goes through the same path

An agent turns an instruction into a directive
([ADR-0042](0042-agent-client-module-and-model-port.md)), the lowering
turns the directive into nodes, and the agent submits a `Revise` intent as
an automation-class principal. The level decides the consent: under
`Assisted` the operator starts each node; under `Supervised` the operator
sees the successor in the status and can veto it before the safe point.
`Unable` and the three checks of ADR-0042 remain the refusal path. The
agent never writes to the lane or to an adapter.

### Planning is shared, not duplicated

The planner for each mission kind (an area survey, a route, a point) lives
in the mission library. A client composes a request, such as an area with
a height and a camera, and submits it with `Issue`. The host plans, admits,
and returns the figures in the receipt. The browser, native, and Apple
clients share the request types and the status mirror through
`pilotage-client-session`; no client plans on its own.

### Component ownership

| Component | Owns | Does not own |
|---|---|---|
| `pilotage-mission-core` | Documents, nodes, conditions (with an operator-consent condition), successor admission and the switch at a node boundary, lifecycle, evidence events | Navigate, planning, transport, authority |
| `pilotage-mission` | Planners for each mission kind, node-to-plan binding per `FollowPlan` node, guidance, figures | Node transitions, authority, transport |
| `pilotage-protocol` | Mission status, intents, receipts, request types on the wire | Execution |
| `pilotage-authority` | The mission scope and its checks | Mission content |
| Session host | The mission lane per vehicle, the bootstrap from the environment, status publication, recovery as a host policy | Planning algorithms, client composition |
| `pilotage-client-session` | The mission object, request composition types, intent submission, status mirror, reconnect by digest | Execution |
| Clients | The composer on the map, the mission panel, consent | Planning, execution |
| Agent module | Directive to nodes through the lowering, `Revise` intents | Writing to the lane or an adapter |
| Adapters | Capability declarations: velocity, heading, return, land, battery, landed, link-loss | Mission logic |

## Consequences

- New wire messages: mission status, intents, receipts, and request types;
  a new scope of authority, the mission scope.
- The mission principal gains the lane with an `Idle` state; the
  environment-variable mission becomes one source of a document.
- The survey planner emits one node per line and one for the way home; the
  flight handler binds a plan per node.
- The App gains a read-only mission panel first, then intents, then the
  composer and consent.
- Adapters declare return and land actions, battery and landed state, and
  a link-loss policy as capabilities; the DJI adapter advertises none of
  them for `Autonomous` until they are flight-verified.
- Sequencing, each step flyable on its own: the status with the waypoint
  node view; intents for the behaviours that exist (`Hold`, `Resume`,
  `Return`, `Abort`, `Start`) with receipts; node-granular survey documents
  with per-node plan binding; successors (`Revise`, `Skip`) at node
  boundaries; `Issue` from a client with the planner's figures in the
  receipt; `Assisted` consent and `Autonomous` recovery behind capability
  gates; the agent through the lowering. Flight verification of each step
  precedes the next.
