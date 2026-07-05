# Pilotage

Rust-first, engine-independent platform for low-latency remote control, supervision,
simulation, and training of maritime, aerial, and terrestrial vehicles.

**Status:** architecture phase. The decision records under [docs/adr](docs/adr/README.md)
are the authoritative design; no implementation code has landed yet.

## What v1 delivers

A browser client controlling a server-hosted Gazebo simulation: rendered video and
telemetry stream down over WebTransport, sequenced control frames stream up, and channel-level
control authority (vehicle helm, camera helm, payload helm) is enforced with scoped
leases and fencing generations. The same binaries run over loopback or LAN for local
demonstrations.

## What the architecture protects

- **Portable sans-IO core.** All protocol, authority, input, timing, and replay logic
  lives in portable Rust crates shared by the browser (WebAssembly) and future native
  clients. Platform code stays thin; domain logic is never duplicated.
- **Engine independence.** Gazebo is the first adapter, not the platform boundary.
  Unreal, Unity, deterministic headless trainers, and real-vehicle gateways are peer
  implementations of the same adapter contract.
- **No mandatory center.** Central services may provide identity, rendezvous, or relay,
  but a session must never require a centrally operated simulator or vehicle fleet.
- **Authority correctness.** Explicit state machines govern handover, emergency
  override, and link loss; every control frame carries a scope and a fencing
  generation, so stale controllers are invalidated immediately.

## Repository layout

| Path | Contents |
|---|---|
| `docs/adr/` | Architecture decision records (authoritative) |
| `docs/` | Architecture overview and supporting design docs |
| `schemas/` | Protobuf wire-schema sources — the protocol's source of truth |
| `crates/` | Portable sans-IO core crates |
| `hosts/` | Session-host binaries |
| `adapters/` | Simulator and vehicle adapters (Gazebo first, reference-headless alongside) |
| `clients/` | Browser (wasm) and future native front ends |
| `services/` | Identity, rendezvous/signaling, and other optional central services |

Start with [docs/architecture.md](docs/architecture.md), then the
[ADR index](docs/adr/README.md).
