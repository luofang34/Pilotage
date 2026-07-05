# ADR-0008: Engine-independent adapter boundary; Gazebo first, reference adapter always

- Status: Accepted
- Date: 2026-07-05

## Context

Gazebo is the first integration, but control, telemetry, media, timing, and authority
semantics must not depend on Gazebo, Unreal, Unity, any game engine, or one vehicle
model. The same contracts must support headless accelerated simulation and
real-vehicle gateways. Different vehicles expose different commands, telemetry
fields, cameras, update rates, and link-loss actions.

## Decision

- The session host defines canonical host, vehicle, control-scope, telemetry,
  media-source, and lifecycle contracts in `pilotage-adapter-api`. Gazebo is the
  first implementation; other engines, deterministic headless trainers, and
  real-vehicle gateways are peer adapters.
- A **deterministic headless reference adapter** is a v1 deliverable, not future
  work: it anchors protocol, timing, replay, accelerated-training, and conformance
  tests independently of all graphical or commercial engines.

```rust
trait VehicleAdapter {
    fn capabilities(&self) -> AdapterCapabilities;
    fn apply_control(&mut self, frame: ScopedControlFrame) -> ApplyOutcome;
    fn sample_telemetry(&mut self) -> TelemetryBatch;
    fn video_sources(&self) -> Vec<VideoSource>;
    fn set_link_loss_policy(&mut self, vehicle: VehicleId, policy: LinkLossPolicy);
    /// Stepped execution for deterministic and accelerated modes (ADR-0013).
    fn step(&mut self, budget: StepBudget) -> StepOutcome;
}
```

The capability description includes: vehicles present; assignable control scopes per
vehicle; command schemas and units; telemetry fields, units, reference frames, and
expected update rates; camera and rendered-video sources; supported link-loss
actions; execution mode (real-time, stepped, accelerated, deterministic,
render-capable, physically embodied); adapter and schema versions.

### Placement and Gazebo v1

- The Gazebo adapter SHOULD run on the same machine as Gazebo, in-process, over
  Gazebo Transport, or as a local sidecar.
- Rendering capture SHOULD occur as close as practical to the Gazebo rendering
  pipeline to minimize copies and latency.
- The public client protocol MUST NOT expose raw Gazebo, Unreal, Unity, ROS, DDS,
  CAN, MAVLink, or other adapter-native messages as its canonical API.

## Consequences

- Vehicle motion and camera control are advertised as separate scopes.
- The canonical model MUST NOT assume an aircraft-only or car-only vocabulary.
- `ApplyOutcome` reports the simulation tick and whether a command was accepted,
  transformed, constrained, or rejected — required for latency accounting
  (ADR-0009) and replay (ADR-0012).
- Real vehicles advertise that stepping, reset, snapshot, or deterministic replay are
  unsupported rather than being forced into simulator semantics.
