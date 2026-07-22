# ADR-0003: Separate identity, authority, real-time data, media, and host planes

- Status: Accepted
- Date: 2026-07-05

## Context

The system combines authentication, group membership, session admission,
channel-level control ownership, low-latency input, telemetry, video streaming, and
simulator integration. A single undifferentiated service or protocol would couple
slow management operations to the real-time path and would obstruct future
peer-hosted deployment. The platform also needs a clean split between globally or
organizationally managed functions and session-local functions running next to the
active simulator or vehicle gateway.

## Decision

Five responsibility planes are defined. They are **contract boundaries, not process
boundaries** — how many binaries or services implement them is a deployment decision.

1. **Identity and admission plane** — passkey/WebAuthn authentication; user,
   organization, and group membership; session discovery and admission; issuance of
   short-lived session capabilities.
2. **Authority plane** — ownership of independently assignable control scopes;
   handover, takeover requests, emergency override, revocation; lease generations and
   effective authority; per-vehicle link-loss policy selection, engaged and cleared
   per scope (ADR-0010).
3. **Real-time data plane** — control frames, fast telemetry, reliable authority and
   mode events, capability and configuration exchange.
4. **Media plane** — rendered-video capture, encoding and delivery, camera-source
   selection, congestion and quality adaptation, timing correlation with telemetry
   and control.
5. **Session host** — simulator process(es) or vehicle gateway, rendering capture,
   vehicle adapter, session-local real-time endpoint, host health and capability
   publication.

v1 ships exactly two deployables: a small identity/signaling service, and the session
host, which embeds the authority engine (ADR-0006), the real-time data plane, and the
media plane.

```text
Passkey authentication
        |
        v
Identity/admission ----> short-lived session capability
        |
        v
Browser <==== HTTPS bootstrap / WebTransport ====> Session host
                                                  |- authority engine
                                                  |- Gazebo + renderer capture
                                                  |- vehicle adapter
                                                  `- telemetry/control/media endpoint
```

## Consequences

- The real-time path MUST NOT query an external identity or policy service per input
  frame; per-frame verification uses compact session-local state.
- Authoritative lease state MUST live at the session host or a directly connected
  authority endpoint (placement decided in ADR-0006).
- The session host MUST expose a versioned capability description.
- Correlation identifiers and monotonic timestamps MUST cross the authority, data,
  media, and host boundaries (ADR-0009, ADR-0012).
- A peer-hosted deployment MAY use a central identity or rendezvous service, but MUST
  NOT require a centrally operated simulator fleet.

## Alternatives considered

- **Single monolithic session gateway:** simple to start, but couples identity,
  video, control, and simulator integration, and blocks peer hosting.
- **One microservice per crate:** rejected; source-code modularity and process
  topology are separate decisions.
