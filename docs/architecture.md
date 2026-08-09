# Pilotage architecture overview

Pilotage is a Rust-first, engine-independent platform for low-latency control,
supervision, simulation, and training of maritime, aerial, and terrestrial
vehicles. A vehicle-side **host** composes flight control, navigation,
surveillance, aeronautical context, and communication adapters. An **operator
client** can be a full control terminal or an EFB preflight tool. An optional
**coordination server** supplies identity, rendezvous, and entitlements. This
document is the orientation map. The [ADRs](adr/README.md) contain the
authoritative decisions.

## System shape (target)

```text
               Optional coordination server (ADR-0027)
     identity/admission · host registry · rendezvous · entitlements
    (never carries session data; optional — local deployments need none)
               |                                |
        HTTPS bootstrap                registration/signaling
               |                                |
   Operator client(s) <═ WebTransport (QUIC) ═> Pilotage host
   (web / iPadOS /      │  direct; opt-in       ├─ authority engine (leases, generations)
    native; plugin      │  separately           ├─ vehicle adapter ── FC (Aviate | PX4)
    panels/layout,      │  deployed relay       ├─ Navigate: navigation solution and guidance
    ADR-0029; EFB or    │                       ├─ Surveillance: traffic fusion and tracks
    terminal posture    │                       ├─ AeroContext: weather, notices, and navdata
    by discovery,       │                       └─ telemetry/control/media/advisory
    ADR-0026)           │
      video ◄───────────┤
      telemetry ◄───────┤
      advisory ◄────────┤
      control ──────────►
      authority events ◄►
```

Clients and hosts are not one-to-one: several operators may hold different
scopes on one vehicle, one client may attach a roster of vehicles, and a
coordinator host aggregates a swarm behind ordinary scopes
([ADR-0028](adr/0028-multi-vehicle-and-swarm-coordinator-hosts.md)). The host
flies with no client attached: the mission executor is an automation-class
principal under the same authority machinery
([ADR-0025](adr/0025-client-optional-operation-automation-principals.md)).

The v1 hosted deployment ships two deployables (identity/signaling service +
session host) and runs the same binaries over loopback or LAN with no special
integration path ([ADR-0004](adr/0004-host-oriented-topology.md)).

## The five planes

| Plane | Owns | Decided in |
|---|---|---|
| Identity & admission | Passkeys, membership, session capabilities | [ADR-0003](adr/0003-separate-responsibility-planes.md), [ADR-0006](adr/0006-capability-auth-scoped-leases-fencing.md) |
| Authority | Scoped leases, fencing generations, handover/override state machines | [ADR-0006](adr/0006-capability-auth-scoped-leases-fencing.md), [ADR-0010](adr/0010-authority-state-machines.md) |
| Real-time data | Control frames, fast telemetry, authority events, bulk config, advisory context | [ADR-0005](adr/0005-webtransport-primary-transport.md), [ADR-0011](adr/0011-message-classes-and-channel-semantics.md), [ADR-0023](adr/0023-vehicle-side-decomposition-fc-navigate-communicate.md) |
| Media | Capture, encode, delivery, adaptation, timing correlation | [ADR-0005](adr/0005-webtransport-primary-transport.md), [ADR-0016](adr/0016-codec-pluggable-media-plane.md) |
| Session host | Simulator/vehicle gateway, adapter, real-time endpoint | [ADR-0004](adr/0004-host-oriented-topology.md), [ADR-0008](adr/0008-engine-independent-adapter-boundary.md) |

Planes are contract boundaries; deployables are a deployment decision.

## System components

| Component | Role | Decided in |
|---|---|---|
| Flight control (Aviate first; PX4 via adapter) | Control-grade estimation, stabilization, actuation | [ADR-0008](adr/0008-engine-independent-adapter-boundary.md), [ADR-0018](adr/0018-avionics-telemetry-and-aviate-adapter.md), [ADR-0024](adr/0024-navigation-authority-boundary.md) |
| Navigate (sibling repository) | Multi-sensor fusion, integrity, flight-plan execution, guidance, terrain awareness | [ADR-0023](adr/0023-vehicle-side-decomposition-fc-navigate-communicate.md), [ADR-0024](adr/0024-navigation-authority-boundary.md) |
| `aero-link` and `avionics-link` | Source access, protocol decode, and thin domain adapters | [ADR-0035](adr/0035-source-neutral-situational-services.md) |
| Surveillance (sibling repository) | Source-neutral traffic observations, fusion, tracks, deltas, and snapshots | [ADR-0035](adr/0035-source-neutral-situational-services.md) |
| AeroContext (repository `v99n62`) | Weather, NOTAM, TFR, briefing, navigation data, revision, validity, and expiry | [ADR-0035](adr/0035-source-neutral-situational-services.md) |
| Pilotage host | Component orchestration + session/authority/media endpoint | [ADR-0003](adr/0003-separate-responsibility-planes.md), [ADR-0004](adr/0004-host-oriented-topology.md), [ADR-0023](adr/0023-vehicle-side-decomposition-fc-navigate-communicate.md) |
| Operator client | Control terminal ↔ EFB by discovered capability; plugin displays | [ADR-0026](adr/0026-host-capability-profiles.md), [ADR-0029](adr/0029-panel-layout-look-plugins.md) |
| Coordination server (optional) | Identity, host registry, rendezvous, entitlement-gated data services | [ADR-0027](adr/0027-optional-coordination-server.md) |

## Situational services

[ADR-0035](adr/0035-source-neutral-situational-services.md) assigns traffic
state to Surveillance and advisory product state to AeroContext. `aero-link`
and `avionics-link` supply source data through thin adapters. Pilotage reads
the domain outputs through a read-only `SituationView`.

Map adapters and AI are optional consumers of `SituationView`. A headless
deployment does not need either consumer. `Communicate` does not own
Surveillance or AeroContext. Add a shared communication mechanism to it only
when two components need that mechanism.

## Load-bearing principles

1. **Sans-IO core** ([ADR-0002](adr/0002-cargo-workspace-portable-sans-io-core.md)):
   all domain logic is pure state machines fed messages and explicit timestamps.
   Browser, native, host, and tests drive the same code; deterministic replay is a
   property, not a feature.
2. **Fencing generations** ([ADR-0006](adr/0006-capability-auth-scoped-leases-fencing.md)):
   authority changes advance a per-scope generation, and the host rejects frames
   from any other generation — a displaced controller is fenced out even if its
   connection stays up.
3. **Engine independence** ([ADR-0008](adr/0008-engine-independent-adapter-boundary.md)):
   Gazebo is adapter #1; the deterministic headless reference adapter ships in v1 and
   anchors conformance.
4. **Explicit time** ([ADR-0009](adr/0009-time-model-and-latency-budget.md)):
   `transport_time`, `host_time`, and `simulation_time` are distinct; staleness is
   rejected, queues are bounded, drops are counted.
5. **Schema-first protocol** ([ADR-0014](adr/0014-protobuf-wire-schema.md)):
   `schemas/` (protobuf) is the source of truth; hosts and clients evolve
   independently under mechanical breaking-change detection.
6. **Advisory and measurement never blur** ([ADR-0023](adr/0023-vehicle-side-decomposition-fc-navigate-communicate.md)):
   measurements ride stamped, coherence-gated telemetry
   ([ADR-0018](adr/0018-avionics-telemetry-and-aviate-adapter.md)); advisory
   context rides provenance- and freshness-tracked classes; neither is ever
   presented as the other.
7. **One authority machinery for every commander**
   ([ADR-0025](adr/0025-client-optional-operation-automation-principals.md)):
   humans, the mission executor, coordinators, and AI agents are all fenced
   principals under the same leases, watchdogs, and audit — no privileged
   side door.

## Implementation increments

| # | Deliverable | Acceptance signal |
|---|---|---|
| 0 | Workspace + quality gates + protocol skeleton + deterministic reference adapter + conformance harness | Client core and test host exchange fixture sessions; CI gates green |
| 1 | Local Gazebo loop: session host, Gazebo adapter, one video source, one vehicle, browser gamepad input; media-over-WebTransport spike (WebCodecs decode, jitter buffer, encoder rate control) | Browser controls local Gazebo over loopback with measured per-stage timing |
| 2 | Server-hosted demo: deployable host, HTTPS bootstrap, reachable QUIC endpoint | Remote browser receives video and controls Gazebo under defined network profiles |
| 3 | Channel scopes: separate motion and camera leases, independent users | One user drives while another controls the camera; stale-scope frames rejected |
| 4 | Normal handover: offer/accept commit + positive confirmations | No ambiguous holder under delayed, duplicated, or reordered events |
| 5 | Override and failure: emergency override, revocation, link-loss policy | Previous generation ineffective immediately; configured failover executes |
| 6 | Peer-host preparation: self-contained host package, registration, direct + relay paths | A non-platform-operated host creates a session without central simulator scheduling |
| 7 | Recording and replay: structured authority/timing log, deterministic replay | A recorded session reproduces authority transitions and applied-control ordering |

### Next increments (component build-out)

Increment 8 is committed as the next slice; the order beyond it is indicative.

| # | Deliverable | Acceptance signal |
|---|---|---|
| 8 | Navigate skeleton: new repository with a sans-IO fusion/flight-plan core; flight-plan execution flies Aviate SITL through the FC's declared setpoint surface as an automation-class principal ([ADR-0023](adr/0023-vehicle-side-decomposition-fc-navigate-communicate.md), [ADR-0024](adr/0024-navigation-authority-boundary.md), [ADR-0025](adr/0025-client-optional-operation-automation-principals.md)); the FC-side guidance command-surface RFC below is a prerequisite | A preloaded plan flies headless in SITL with no client attached; a joining client sees automation as holder and takes over via the authority machinery |
| 9 | Authority completion: handover/override wire vocabulary, identity/admission service, observer admission | Two operators transfer a scope with positive confirmation; a supervisor overrides; a monitor observes with no grantable scopes |
| 10 | EFB slice: client embeds AeroContext cores; briefing on a live map; data-gateway host profile ([ADR-0026](adr/0026-host-capability-profiles.md)) | Preflight brief and pack-for-flight on a client with no host process; the same client is a full terminal against a full-authority host |
| 11 | Coordination server: registry + rendezvous + entitlement gate ([ADR-0027](adr/0027-optional-coordination-server.md)) | A WAN session forms behind NAT with no session data transiting the server |
| 12 | Coordinator host: aggregate scopes decomposed over member hosts ([ADR-0028](adr/0028-multi-vehicle-and-swarm-coordinator-hosts.md)) | A swarm command reaches members under end-to-end fencing; displacing the coordinator on one member affects exactly that member |

## Backlog

### P0 — resolve before the corresponding increment freezes

- Certificate strategy for local and peer-hosted hosts: locally provisioned dev
  certificate vs `serverCertificateHashes` (Safari support unverified)
  (increment 1).
- Video bandwidth-adaptation and keyframe-recovery strategy validated under
  impaired networks (increments 1–2). Browser floor is already set by ADR-0005:
  WebTransport-capable browsers, i.e. Safari 26.4+/iOS 26.4+/iPadOS 26.4+,
  Chrome 97+, Edge 98+, Firefox 114+.
- p95/p99 closed-loop latency targets under specified network profiles (after
  increment 2 measurements).
- Expected simultaneous operators and spectators per host (increment 3).
- Emergency-override authority-class matrix and takeover-veto policy, now
  including automation, agent, and aggregate-scope rows (increments 5, 8, 12).
- Host registration and trust model; threat model for malicious hosts, clients, and
  compromised tokens (increment 6).
- Recording retention and privacy policy (increment 7).
- Aiding-observation schema RFC with the Aviate side — frames, covariance,
  integrity terms, source composition ([ADR-0024](adr/0024-navigation-authority-boundary.md))
  — before any aiding lands (increment 8 exit).
- Guidance command-surface RFC with the Aviate side: position/velocity
  setpoint ingress mapped into the FC command path, and deviation-setpoint
  carriage and consumption defined
  ([ADR-0024](adr/0024-navigation-authority-boundary.md)) (increment 8
  prerequisite).
- Loss-of-communication procedure configuration: format, provenance, and
  selection rules ([ADR-0025](adr/0025-client-optional-operation-automation-principals.md))
  (increment 8).
- Panel descriptor and extensible state-group contract design, and the
  safety-fixed vs skinnable attribute set
  ([ADR-0029](adr/0029-panel-layout-look-plugins.md)) — before plugin work
  begins.

### P1 — design now, implement after the core path is stable

Multiple camera sources and picture-in-picture;
spectator stream fan-out (host- or relay-side replication); instructor and supervisory modes; organization
policy and temporary guests; automation-assisted blended control; signed online
device-registry updates; repeatable network-impairment benchmark harness; peer-host
update and attestation; FIS-B providers in AeroContext; traffic providers in
Surveillance; host-to-host
collaboration behaviors over the reserved coordinator seam.

### P2 — preserve compatibility, defer implementation

Haptics; VR and head tracking; vendor-specific joystick extensions; spatial audio;
advanced SVC and spectator quality tiers; tournament/adjudication tooling.

## Validation matrix

| Area | Required validation |
|---|---|
| Client input | Device enumeration, hot-plug, calibration, focus loss, background throttling, multiple devices |
| Authority | Per-scope isolation, stale-generation rejection, handover races, override races, duplicate acknowledgement |
| Network | Delay, jitter, loss, reordering, NAT, QUIC relay, UDP-blocked fallback, connection migration, loopback |
| Media | Capture latency, encoder queueing, bitrate collapse, keyframe recovery, decode and presentation |
| Simulator | Tick delay, renderer slowdown, adapter restart, vehicle respawn, dynamic camera addition |
| Host lifecycle | Registration, update, reconnect, crash recovery, peer-host trust and revocation |
| Link loss | Per-vehicle configuration, default inheritance, stale-input hold limit, neutralization or automation transition |
| Navigation | Fusion integrity honesty, single-source degradation, FC/Navigate divergence display, aiding rejection paths |
| Autonomy | Headless plan execution, link-loss automation engagement, human takeover and give-back, stalled-automation fencing |
| Multi-vehicle | Roster attach, coordinator displacement per member, member link-loss independence, media keyed by session |
| Capability profiles | Host-absent bootstrap, data-gateway honesty (zero actuator scopes), EFB↔terminal continuity |
| Observability | Correlation from client sample through simulator application to resulting telemetry/video |
