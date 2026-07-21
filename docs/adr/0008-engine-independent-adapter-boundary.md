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
    // Link-loss is enacted PER SCOPE and is fallible — see Amendments below.
    fn set_link_loss_policy(
        &mut self,
        vehicle: VehicleId,
        scope: &ScopeId,
        policy: Option<LinkLossPolicy>,
    ) -> Result<(), LinkLossEnactError>;
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

## Amendments

### 2026-07-21 — Link-loss enactment is per-scope and fallible

The original `set_link_loss_policy(vehicle, policy)` was vehicle-wide: any
scope's holder loss drove the whole vehicle to one failover state. That is
unsafe once a vehicle carries independently-leased scopes (e.g. `vehicle.motion`
and `vehicle.gimbal`): losing or releasing the gimbal would brake the aircraft.

The contract is now **per scope**:

- `set_link_loss_policy(vehicle, scope, policy)` engages (`Some`) or clears
  (`None`) the policy for **one scope only**. While a scope's policy is engaged
  the adapter MUST reject that scope's ordinary control frames
  (`RejectReason::LinkLossEngaged`) and MUST drive **only that scope's** actuation
  toward its safe state — motion neutralizes the flight setpoint, the gimbal
  queues a zero-rate stop, and neither reaches the other. `None` is the only path
  back to normal control for that scope.
- The call is **fallible** (`Result<(), LinkLossEnactError>`) with an
  **asymmetric, always fail-closed postcondition**:
  - **Engage (`Some`)**: the latch is recorded and the scope stays suppressed
    **even if the actuation is refused** (`Err`) — a fenced scope must never be
    left drivable, so a failed engage still rejects that scope's control.
  - **Clear (`None`)**: the latch is removed and the scope returns to normal
    control **only on `Ok`**. On `Err` the latch **stays engaged** (the scope
    keeps rejecting control) and the driver retries; a scope is un-suppressed
    only by a clear the adapter accepted, never speculatively.
  A refused enactment is a counted fail-closed fault, never a silent no-op.
- Actuation is **best-effort at the link boundary**: an `Ok` return means the
  safe-state command reached the vehicle link, not that the vehicle confirmed it
  (e.g. the PX4 gimbal stop is **queued** without waiting for a `CONFIGURE`
  acknowledgement). The DECLARED independent safety net is the FC's own
  gimbal-manager setpoint-timeout failsafe, which zeroes a nonzero angular rate
  after ~2 s (PX4-Autopilot `src/modules/gimbal/output.cpp`, behavior pinned at
  commit `841bb40`); the host does **not** re-send a stop after link loss
  (`queue_link_loss_stop` stops the stream, so the host's stale-demand cutoff
  does not fire). Adapters that CAN confirm (a real gateway tracking device
  acknowledgement) SHOULD keep the scope engaged/faulted until the vehicle
  confirms the safe state.
- The **supported-actions menu remains per vehicle** (see ADR-0010): configuration
  selects one policy per vehicle from the menu it advertises; that selected policy
  is what each of the vehicle's scopes engages independently on its own holder loss.

Evidence: per-scope suppression, the queued best-effort gimbal zero-rate stop, and
a failed clear that STILL rejects control are covered by the PX4 adapter's
`gimbal_link_loss_tests` and the `engine_actor` stateful-adapter test; the host-side
per-scope engage/clear, the ack-after-confirmed-clear, the generation-gated retry of
a refused clear, and the handover/override invalidation are covered in
`session-host`'s `engine_actor` and `pilotage-session`'s `recovery` tests. The
independent PX4 rate-zeroing fallback is a MANUAL acceptance criterion of the
`cargo xtask sim px4-gz` bring-up, made DISCRIMINATING by fault injection
(`PILOTAGE_PX4_DROP_GIMBAL_STOP=1` drops the host's stop so PX4's timeout is the
sole failsafe under test). It was exercised on 2026-07-21 against PX4
`6120aa53` (`v1.18.0-beta1-110-g6120aa53df`): with the gimbal under Pilotage's
primary control, a holder disconnect reproducibly logged the host DROPPING its
stop. That proves only the Pilotage half (no stop was sent); the PX4 half — the
gimbal keeping its rate and PX4 zeroing it ~2 s later — is code-verified but not
yet observed on the wire, so the outcome is pending (the rate-vs-time trace that
would close it is tracked by #168). Full record and the exact SHA in
`tools/xtask/src/backend/px4_gz.rs`. No automated PX4-in-the-loop test runs in CI.
