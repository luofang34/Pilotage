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

An agent never calls a domain or an adapter to act. It submits an intent,
such as "change the destination" or "request a hold", as an automation-class
principal (ADR-0025). The authority engine checks the intent against the
agent's scopes and leases, like an intent from a person. Escalation rules
apply.

### Performance

- The port returns snapshots by reference to immutable records. It does not
  copy domain state for each query.
- An agent can subscribe to a domain and receive revisions. It does not poll.
- Each answer names the snapshot revisions it used, so an agent can cache
  results until a revision changes.

## Consequences

- One context serves every agent location. An agent tested against a client
  session works on the onboard host.
- The model port of ADR-0042 does not change. This decision fixes what the
  agent reads and how it acts.
- The port surface and the intent vocabulary are tracked as separate work.
