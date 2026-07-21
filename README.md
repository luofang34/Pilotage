# Pilotage

Engine-independent platform for low-latency remote control, supervision, simulation,
and training of maritime, aerial, and terrestrial vehicles.

> **⚠️ Work in progress — experimental.** Pilotage is early-stage, experimental
> software under active development, provided **as is** with **no warranty or
> guarantee of any kind**, express or implied — including, without limitation,
> fitness for a particular purpose, correctness, reliability, availability, or
> safety. Interfaces, wire formats, and behavior may change without notice.
>
> **SIM / NOT FOR FLIGHT.** Nothing here is certified, approved, or airworthy.
> Nothing may be used for operational control of a real vehicle or for any
> safety-critical purpose. Use at your own risk.

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

## Run a PX4 SITL session

One command launches a PX4 + Gazebo software-in-the-loop session and opens the
browser viewer:

```sh
cargo xtask sim --fc px4-gz
```

The launcher builds what a fresh checkout is missing before it starts: the
release session host and the viewer's generated wasm runtime are built
automatically on the first run, and the C++ gz camera sidecar is built
best-effort (the session still flies without it — you just get no video).

You provide the pieces git does not vendor:

- A **PX4-Autopilot** SITL checkout, built once (`make px4_sitl`), at
  `../PX4-Autopilot` or the path in `PX4_DIR`. The launcher fails with the
  exact missing path when it is absent.
- **Gazebo Harmonic** (`gz`) on `PATH`.
- Toolchains for the auto-built artifacts: `wasm-bindgen-cli` 0.2.126 plus the
  `wasm32-unknown-unknown` target (viewer, required), and `protobuf` + the
  Gazebo dev libraries (camera sidecar, optional). Each build script prints its
  own install hint on failure.

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
