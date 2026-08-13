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
    native; modules     │  separately           ├─ vehicle adapter ── FC (Aviate | PX4)
    selected from       │  deployed relay       ├─ Navigate: navigation solution and guidance
    source catalog,     │                       ├─ Surveillance: traffic fusion and tracks
    platform ports,     │                       ├─ Airmass and AeronauticalUpdates
    and authorization,  │                       ├─ Navdata, FlightPlanning, and Briefing
    ADR-0037)           │                       └─ telemetry/control/media/advisory
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
| `aero-link` | Radio access, protocol decode, bounded FIS-B product assembly, classification, and domain adapters | [ADR-0036](adr/0036-situational-domain-ownership.md) |
| `avionics-link` | Installed-avionics transport, protocol decode, and domain adapters | [ADR-0036](adr/0036-situational-domain-ownership.md) |
| Surveillance (sibling repository) | Source-neutral traffic observations, fusion, tracks, deltas, and snapshots | [ADR-0036](adr/0036-situational-domain-ownership.md) |
| Airmass | Weather observations, forecasts, hazards, revisions, validity, and expiry | [ADR-0036](adr/0036-situational-domain-ownership.md) |
| `AeronauticalUpdates` | Perishable notices, restrictions, operational status, validity, and expiry | [ADR-0036](adr/0036-situational-domain-ownership.md) |
| Navdata | Cycle-dated NASR and CIFP baseline | [ADR-0036](adr/0036-situational-domain-ownership.md) |
| FlightPlanning | Plan drafts, resolution, validation, filing state, and immutable revisions | [ADR-0036](adr/0036-situational-domain-ownership.md) |
| Briefing | Immutable evidence results from fixed inputs | [ADR-0036](adr/0036-situational-domain-ownership.md) |
| AeroContext (repository `v99n62`) | Temporary compatibility facade for existing consumers | [ADR-0036](adr/0036-situational-domain-ownership.md) |
| Pilotage host | Composition, session and authority services, media endpoint, and read-only `SituationView` | [ADR-0003](adr/0003-separate-responsibility-planes.md), [ADR-0004](adr/0004-host-oriented-topology.md), [ADR-0023](adr/0023-vehicle-side-decomposition-fc-navigate-communicate.md), [ADR-0036](adr/0036-situational-domain-ownership.md) |
| Operator client | Shared function modules selected from source data, platform ports, and authorization | [ADR-0037](adr/0037-modular-operator-client-composition.md), [ADR-0029](adr/0029-panel-layout-look-plugins.md) |
| Coordination server (optional) | Identity, host registry, rendezvous, entitlement-gated data services | [ADR-0027](adr/0027-optional-coordination-server.md) |

## Situational services

[ADR-0036](adr/0036-situational-domain-ownership.md) assigns each type of
situational state to one lifecycle owner. Surveillance owns traffic state.
Airmass owns weather state. `AeronauticalUpdates` owns perishable notices and
operational status. Navdata owns the cycle-dated baseline. FlightPlanning owns
plan state. Briefing owns immutable results from fixed inputs.

`aero-link` and `avionics-link` supply source data through domain adapters.
Pilotage composes immutable snapshot handles through a read-only
`SituationView`.

Map adapters and AI are optional consumers of `SituationView`. A headless
deployment does not need either consumer. An application owns presentation,
tiles, and styling. `Communicate` does not own situational domain state. Add a
shared communication mechanism to it only when two components need that
mechanism.

The [domain snapshot envelope contract](domain-snapshot-envelope.md) defines
immutable domain output. It defines producer identity, snapshot revision,
retainable handles, field evidence, absence, and schema-version rules.

The [SituationView V1 contract](situation-view-contract.md) defines the
versioned query and result. It defines clock correspondence, best-available
consistency, age assessment, and the shared conformance corpus.

## Operator client composition

[ADR-0037](adr/0037-modular-operator-client-composition.md) defines one modular
operator-client architecture. The web and iPadOS user interfaces can use
different layouts and module sets. The installed modules use shared semantic
cores and versioned data contracts.

The source catalog, installed platform ports, and authorization state determine
which modules are available. A local adapter and a remote host feed the same
typed module inputs. The architecture decision contains the durable composition
graph and the read-only iPadOS instrument slice.

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

The GitHub architecture project controls execution order. This table shows
capability dependencies.

| # | Deliverable | Acceptance signal |
|---|---|---|
| 8 | Navigate skeleton: new repository with a sans-IO fusion/flight-plan core; flight-plan execution flies Aviate SITL through the FC's declared setpoint surface as an automation-class principal ([ADR-0023](adr/0023-vehicle-side-decomposition-fc-navigate-communicate.md), [ADR-0024](adr/0024-navigation-authority-boundary.md), [ADR-0025](adr/0025-client-optional-operation-automation-principals.md)); the FC-side guidance command-surface RFC below is a prerequisite | A preloaded plan flies headless in SITL with no client attached; a joining client sees automation as holder and takes over via the authority machinery |
| 9 | Authority completion: handover/override wire vocabulary, identity/admission service, observer admission | Two operators transfer a scope with positive confirmation; a supervisor overrides; a monitor observes with no grantable scopes |
| 10 | Modular operator client: source catalog, portable client-session core, and read-only iPadOS instruments while the local situation module stays host-optional ([ADR-0037](adr/0037-modular-operator-client-composition.md)) | The iPadOS client renders live instruments as an observer; one telemetry fixture gives the same canonical state and scene identity on web and Apple paths; the local situation view works with no host |
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
