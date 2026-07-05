# Pilotage architecture overview

Pilotage is a Rust-first, engine-independent platform for low-latency control,
supervision, simulation, and training of maritime, aerial, and terrestrial vehicles.
This document is the orientation map; the [ADRs](adr/README.md) are the authoritative
decisions.

## System shape (v1 hosted deployment)

```text
                 Passkey / WebAuthn
                        |
                        v
              Identity & admission service ──> short-lived session capability
                        |
             session bootstrap (HTTPS)
                        |
   Browser client <═ WebTransport (QUIC) ═> Session host
   (wasm core +        │                     ├─ authority engine (scoped leases, generations)
    browser ports)     │                     ├─ Gazebo + renderer capture
                       │                     ├─ vehicle adapter
      video ◄──────────┤                     └─ telemetry / control / media endpoint
      telemetry ◄──────┤
      control ─────────►
      authority events ◄──►
```

The same two deployables run a local demonstration over loopback or LAN with no
special integration path. Peer-hosted simulators and real-vehicle gateways are future
session hosts behind the same contracts; central services stay optional
([ADR-0004](adr/0004-host-oriented-topology.md)).

## The five planes

| Plane | Owns | Decided in |
|---|---|---|
| Identity & admission | Passkeys, membership, session capabilities | [ADR-0003](adr/0003-separate-responsibility-planes.md), [ADR-0006](adr/0006-capability-auth-scoped-leases-fencing.md) |
| Authority | Scoped leases, fencing generations, handover/override state machines | [ADR-0006](adr/0006-capability-auth-scoped-leases-fencing.md), [ADR-0010](adr/0010-authority-state-machines.md) |
| Real-time data | Control frames, fast telemetry, authority events, bulk config | [ADR-0005](adr/0005-webtransport-primary-transport.md), [ADR-0011](adr/0011-message-classes-and-channel-semantics.md) |
| Media | Capture, encode, delivery, adaptation, timing correlation | [ADR-0005](adr/0005-webtransport-primary-transport.md) |
| Session host | Simulator/vehicle gateway, adapter, real-time endpoint | [ADR-0004](adr/0004-host-oriented-topology.md), [ADR-0008](adr/0008-engine-independent-adapter-boundary.md) |

Planes are contract boundaries; v1 ships them as two deployables (identity/signaling
service + session host).

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
- Emergency-override authority-class matrix and takeover-veto policy (increment 5).
- Host registration and trust model; threat model for malicious hosts, clients, and
  compromised tokens (increment 6).
- Recording retention and privacy policy (increment 7).

### P1 — design now, implement after the core path is stable

WebTransport for control/telemetry; multiple camera sources and picture-in-picture;
spectator stream fan-out (host- or relay-side replication); instructor and supervisory modes; organization
policy and temporary guests; automation-assisted blended control; signed online
device-registry updates; repeatable network-impairment benchmark harness; peer-host
update and attestation.

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
| Observability | Correlation from client sample through simulator application to resulting telemetry/video |
