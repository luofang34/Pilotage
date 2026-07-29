# ADR-0027: Optional coordination server: identity, rendezvous, and entitlements — never the session data plane

- Status: Proposed
- Date: 2026-07-29

## Context

WAN deployments need authentication and authorization, discovery and
rendezvous across NAT, fleet inventory, and access to vendor-credentialed
data services (FSS briefing requests, and eventually flight-plan filing).
The services area reserves an identity/admission service.
[ADR-0003](0003-separate-responsibility-planes.md) and
[ADR-0004](0004-host-oriented-topology.md) keep central services optional
in a narrow sense — no centrally operated simulator fleet, no centrally
scheduled compute; this record strengthens that stance for the server
role. aerocontext ships a working embryo of the entitlement arm: a proxy
daemon holding vendor secrets server-side, gating calls on
verifiable-credential presentations (the aerocred stack). A local
deployment must work with no server configured at all.

## Decision

- One optional **coordination server** role composes four functions:
  1. **Identity and admission** — passkey/WebAuthn authentication,
     membership, short-lived session capabilities
     ([ADR-0003](0003-separate-responsibility-planes.md),
     [ADR-0006](0006-capability-auth-scoped-leases-fencing.md)).
  2. **Host registry and fleet inventory** — registration, capability
     advertisement, and fleet views; fleet orchestration remains outside
     the session data-plane correctness boundary
     ([ADR-0004](0004-host-oriented-topology.md)).
  3. **Rendezvous** — brokering direct client↔host QUIC path
     establishment: endpoint discovery, address exchange, and hole-punch
     coordination.
  4. **Entitlement-gated data services** — vendor-credentialed providers
     behind verifiable-credential presentations: airman eligibility, plus
     a paid entitlement where the service policy requires one; the
     aerocontext proxy/aerocred stack is this arm. FSS briefing request is
     the first such service. Flight-plan filing joins when it lands — in
     aerocontext it is a deliberately stubbed surface behind its command
     safety boundary, and its command-class nature also implicates the
     escalation machinery of
     [ADR-0025](0025-client-optional-operation-automation-principals.md),
     not the entitlement gate alone.
- **Identity has a home in every deployment.** The server hosts the
  identity/admission plane for WAN deployments; with no server configured,
  the host itself issues session capabilities under its local policy —
  planes are contract boundaries
  ([ADR-0003](0003-separate-responsibility-planes.md)), so the identity
  service MAY be co-resident with the host for local deployments.
- **The server MUST NOT carry session data.** No control, telemetry,
  authority events, or media transit it. Active sessions MUST survive
  server unavailability; local and LAN sessions MUST form with no server
  configured. This is a new, stronger commitment than the accepted
  records make, and it is this record's core decision.
- **NAT fallback is an opt-in, separately deployed relay.** When no direct
  path can be established, a MASQUE-style QUIC relay
  ([ADR-0004](0004-host-oriented-topology.md),
  [ADR-0005](0005-webtransport-primary-transport.md)) MAY carry the
  session. The relay is never part of the server role, never required, and
  is chosen explicitly by the deploying operator — a relay's operator sees
  encrypted QUIC, but its availability and throughput become part of the
  session's fate, which is exactly why it is a deliberate deployment
  choice rather than a silent fallback.
- **Feature gating is explicit and partial.** Capabilities that require
  vendor credentials or verified identity are available only through the
  server; their absence degrades those features only, never the session or
  local data.

## Consequences

- Server outage costs new WAN rendezvous and entitlement-gated services;
  active sessions, local sessions, and all locally held data are
  unaffected.
- Host registration, update, and attestation — the trust model for who may
  appear in a fleet — remains the open question tracked in
  [ADR-0004](0004-host-oriented-topology.md).
- The entitlement mechanism reuses the aerocontext proxy/aerocred design
  (server-held vendor secrets, challenge-nonce verifiable presentations)
  rather than inventing a second gate.
- Rendezvous requires the host to maintain a registration channel to the
  server when WAN-reachable operation is desired; the channel carries
  registration and signaling only.

## Alternatives considered

- **Server-relayed data plane (SFU/TURN-style):** rejected; it puts a
  center in the latency path and makes the operator carry sessions — the
  opposite of this record's server-out-of-data-plane decision, and beyond
  even the relay allowance [ADR-0004](0004-host-oriented-topology.md)
  grants.
- **Direct-only with no relay anywhere:** rejected; address-dependent NATs
  on both ends would make some deployments simply impossible, and the
  operator deserves the option to accept a relay knowingly.
- **Folding entitlements into the host:** rejected; vendor secrets must
  not live on vehicles, and entitlement verification is an
  identity-plane concern.
