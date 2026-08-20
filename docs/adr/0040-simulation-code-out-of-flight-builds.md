# ADR-0040: Simulation-only code stays out of flight builds

- Status: Accepted
- Date: 2026-08-18

## Context

The session host embeds simulator adapters, a simulator video sidecar
client, and simulation-truth oracle bindings. A flight deployment does
not use this code. Before this decision, one build of the host carried
all of it, and only runtime checks (profile parsing, capability
advertisement) kept simulation behavior out of a flight session.

Runtime checks control behavior. They do not control what code is in
the binary. A flight build that contains simulator code has a larger
audit surface than the deployment needs.

The workspace had no cargo features. [ADR-0026](0026-host-capability-profiles.md)
says clients adapt by discovery, not by build variants. That decision
is about clients, and it stands. This decision is about the host
binary.

## Decision

- **The host gets one cargo feature: `sim`.** The feature is on by
  default, so development and CI build the full simulation host. A
  flight build uses `--no-default-features`.
- **The `sim` feature carries all simulation-only code:**
  - the Gazebo diff-drive adapter and the reference adapter;
  - `pilotage-sim-video`, the sidecar video client — every simulator
    video producer speaks its protocol, and no physical vehicle does;
  - the PX4 and Aviate adapters' camera-sidecar paths (each adapter has
    its own `sim` feature, wired through the host's);
  - the Aviate XIL truth-oracle bindings (`aviate-xil-contract`,
    `aviate-xil-shm`) and the simulation profiles that use them. In a
    flight build, the Aviate simulation profiles are structurally
    absent: profile selection returns a typed error, not a degraded
    session.
- **Uninhabited types keep the seams small.** A gated attachment field
  (camera sidecar, truth oracle) keeps its `Option<T>` shape; the
  flight build replaces `T` with an uninhabited type, so the field is
  structurally `None` and the shared code does not fork.
- **Frame identity is not simulation code.** `RawVideoFrame`,
  `UnstampedFrame`, and `FrameStamper` (ADR-0020) live in
  `pilotage-adapter-api`. The media plane serves any video producer,
  physical ones included, so it stays in every build.
- **Clients do not change.** A client discovers what a host offers
  (ADR-0026, ADR-0037). No client build variant exists, and the wire
  protocol is identical in both host builds.
- **The gate is CI-enforced.** `scripts/check-flight-build.sh` builds
  the host with `--no-default-features` and fails if the dependency
  tree contains a simulation-only crate.

## Consequences

- A flight build of the host contains no simulator adapter, no sidecar
  client, and no XIL binding. `cargo tree` proves it, and CI keeps it
  true.
- Development ergonomics do not change: default builds and tests see
  the full simulation surface.
- A new simulation backend (X-Plane, JSBSim, MSFS) adds code only
  behind the `sim` feature; the flight build cannot grow simulator
  code without failing the gate.
- The `--adapter` vocabulary stays stable across builds; a flight
  build refuses a simulation-only adapter with a typed startup error.
