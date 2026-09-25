# ADR-0047: Agents read one context port and act only through authority-checked intents

- Status: Proposed
- Date: 2026-09-23

## Context

ADR-0042 composes an agent as a client module behind a model port. The agent
reads the client session: telemetry, authority events, and navigation
guidance. An onboard host with no client also needs agents. These agents
must know the navigation solution and its integrity, the pending mission,
weather, airspace, traffic, the aircraft state, and the authority state.

If each agent location assembles its own view, an agent on the host and an
agent on a client can see different data for the same question.

## Decision

### The context port

The host library (ADR-0045) exposes one read-only `AgentContext` port. It
gives:

- the `SituationView` snapshots of each domain (ADR-0036),
- the Navigate solution with its integrity assessment,
- the mission state and the pending steps,
- the Aircraft domain state (ADR-0046),
- the authority state: which principal holds which scope.

Every item carries its stamp, its source, and its validity. An item that is
stale or unavailable says so. The port never fills a gap with a default.

The same port serves an agent on the onboard host and an agent in a client.
In a client, the port reads the host through the session. The agent code
does not change.

### Actions are intents

An **intent** is a typed request from a principal for a change in mission or
vehicle state, such as "change the destination" or "request a hold". The
authority engine checks an intent before a domain or an adapter acts on it.

An agent never calls a domain or an adapter to act. It submits an intent as
an automation-class principal (ADR-0025). The authority engine checks the
intent against the agent's scopes and leases, like an intent from a person.
Escalation rules apply.

A lease request and a discrete action are intents. A typed control frame
under a held lease (ADR-0042) is not a separate intent. The lease that the
authority engine granted covers the frame, and fencing (ADR-0006) checks the
frame.

### Performance

- The port returns snapshots by reference to immutable records. It does not
  copy domain state for each query.
- An agent can subscribe to a domain and receive revisions. It does not poll.
- Each answer names the snapshot revisions it used, so an agent can cache
  results until a revision changes.

## Consequences

- One context serves every agent location. An agent tested against a client
  session works on the onboard host.
- This decision amends ADR-0042. These parts of ADR-0042 stay: the model
  port, the agent module in a client, and its lease, control frames, and
  discrete actions. These parts change:
  - An agent can also run on the onboard host, as an automation-class
    principal of the host library. ADR-0042 rejected a model inside the host
    as a privileged control path. A host agent has no privileged path: the
    authority engine checks each of its intents.
  - An agent reads the `AgentContext` port, not only the typed module inputs
    of the client session.
  - An agent acts only through intents and through frames under a lease that
    an intent got.
- The port surface and the intent vocabulary are tracked as separate work.
