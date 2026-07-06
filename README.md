# Pilotage

Engine-independent platform for low-latency remote control, supervision, simulation,
and training of maritime, aerial, and terrestrial vehicles.

**Status:** early implementation. The core protocol/authority/input crates, a
WebTransport session host, and a Gazebo adapter are in place; a browser and a native
client drive a real Gazebo vehicle end-to-end (video, telemetry, and control). The
decision records under [docs/adr](docs/adr/README.md) are the authoritative design.

## Working demo

A diff-drive vehicle in Gazebo, driven from a browser or native client: the onboard
camera streams down as MJPEG over WebTransport, odometry streams down as telemetry,
and keyboard/gamepad control streams up — with channel-level authority (scoped leases
and fencing generations) enforced on every frame. A C++ gz-transport sidecar bridges
Gazebo to the Rust adapter; a deterministic headless adapter stands in when Gazebo is
not present. Runs over loopback or LAN.

## What the architecture protects

- **Portable sans-IO core.** All protocol, authority, input, timing, and replay logic
  lives in portable crates shared by the browser and native clients. Platform code
  stays thin; domain logic is never duplicated.
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
| `hosts/` | Session-host binary |
| `adapters/` | Vehicle adapters: Gazebo (with its C++ bridge) and the deterministic reference |
| `sim/` | Gazebo demo worlds |
| `clients/` | Browser demo viewer (native viewer lives under `tools/`) |
| `tools/` | Native viewer/probe and the HID device probe |
| `services/` | Identity, rendezvous/signaling, and other optional central services |

Start with [docs/architecture.md](docs/architecture.md), then the
[ADR index](docs/adr/README.md).
