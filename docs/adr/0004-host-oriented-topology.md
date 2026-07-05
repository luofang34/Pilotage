# ADR-0004: Host-oriented topology spanning hosted, peer-hosted, and real-vehicle sessions

- Status: Accepted
- Date: 2026-07-05

## Context

v1 runs Gazebo and its rendering workload on an application-controlled server,
streaming video and telemetry to a browser and receiving control input. The same
implementation must support a local demonstration over loopback or LAN. Beyond v1,
the same abstraction must cover peer-hosted simulators, organization-operated
clusters, and independently operated real vehicles, where the host endpoint may run
onboard, on an edge companion computer, or at a nearby gateway.

## Decision

- A **session host** is the unit of deployment: it runs one or more adapters and the
  session-local real-time media/control endpoint. In v1 it runs Gazebo and renderer
  capture; for a real vehicle it bridges cameras, sensors, actuators, and vehicle
  control networks.
- The first production topology deploys the session host on an application-controlled
  server. A local demonstration uses the same host binary and browser application
  over loopback or LAN; local mode MUST NOT diverge from production behavior through
  a special integration path.
- The session host MUST be versioned, self-contained, and independently deployable.
- Host registration, capability advertisement, and session admission use explicit
  protocols; the browser treats the host endpoint as session-specific, never as a
  permanent central media server.
- Direct client-to-host connectivity SHOULD be used when reachability and policy
  permit; relayed connectivity (a MASQUE-style QUIC relay) MAY be used when a host
  is not directly dialable. A centrally supplied relay does not constitute a central
  simulator fleet.
- The platform operator MUST NOT be required to own or schedule simulator compute or
  vehicle fleets. Fleet orchestration is an optional deployment concern outside the
  session data plane.

```text
Hosted v1                          Peer-hosted (future)

Browser                            Browser
   | HTTPS bootstrap + WebTransport   | direct WebTransport where dialable,
   v                                  v QUIC relay where necessary
Server-hosted session host         User/org-operated session host
   |- Gazebo + render capture         |- simulator or vehicle gateway
   |- vehicle adapter                 |- vehicle adapter
   `- telemetry/control endpoint      `- telemetry/control endpoint

                       Optional central services:
                identity | rendezvous | policy | QUIC relay
```

## Consequences

- One deployment boundary covers simulation, rendering, telemetry, and control across
  local testing, centralized v1 hosting, and decentralized future hosting.
- Certificates, origins, and browser secure-context requirements for local and
  peer-hosted hosts need an explicit strategy (open question below).
- Peer-hosted and real-vehicle operation require a trust model for host software,
  adapter integrity, telemetry authenticity, and actuator safety boundaries.

## Open questions

- Browser secure-origin strategy for local and peer-hosted hosts. Leaning: a
  development origin plus locally provisioned certificates for the local demo;
  decide during the first local-loop increment.
- How hosts are updated, attested, and permitted to join organizational deployments.
