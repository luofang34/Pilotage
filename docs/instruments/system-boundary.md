# Instrument system boundary

## Boundary diagram

```text
UNTRUSTED SOURCES AND TRANSPORTS                 DISPLAY FUNCTION

 simulator truth   USB CDC   future aircraft bus
       |               |              |
       +---------- adapter / decoder --+
                         |
          network / shared memory / replay
                         |
                         v
        +---------------------------------------+
        | input validation and time/integrity  |
        | ordering, range, coherence, compare  |
        +-------------------+-------------------+
                            |
                 atomic DisplaySnapshot
                            |
          +-----------------+------------------+
          |                                    |
          v                                    v
  display/mode/alert model              optional SVS model
          |                                    |
          v                                    v
  critical 2D scene commands          terrain/depth background
          +-----------------+------------------+
                            |
                 explicit-priority compositor
                            |
                 deterministic output backend
                            |
          +-----------------+------------------+
          |                                    |
          v                                    v
     display surface               independent progress/output monitor
          |                                    |
          +---------- failure state <----------+

SIMULATOR PORTS: WebTransport, WASM, Canvas, browser video, Gazebo, harness
AIRCRAFT PROJECT PORTS: sensor/bus adapters, qualified renderer and display,
                       independent monitor, installation-specific I/O
OUTSIDE BOUNDARY: sensors and source estimators, TAWS logic, flight guidance,
                  optical HUD/HWD installation, aircraft alert source logic
```

## Responsibilities inside the boundary

The instrument boundary owns:

- validation of every consumed value and its reference metadata under
  [`AIR-BAS-004`](requirements.md#air-bas-004);
- ordering, age, source-reset, replay, and coherent-snapshot decisions under
  [`AIR-IN-008`](requirements.md#air-in-008) and
  [`AIR-UNAV-004`](requirements.md#air-unav-004);
- per-signal validity, miscompare, and fault reason under
  [`AIR-IN-009`](requirements.md#air-in-009);
- deterministic display modes, reversion, priorities, and command output under
  [`AIR-BAS-007`](requirements.md#air-bas-007);
- visible failure, simulation, and conformality presentation; and
- monitoring that detects a renderer or output path retaining a last-good image
  under [`AIR-IN-013`](requirements.md#air-in-013).

Transport delivery is not evidence that a measurement is fresh or trustworthy.
Adapters and transports may duplicate, delay, reorder, replay, corrupt, or
re-time data. They remain outside the trusted display core until input evidence
passes the relevant checks.

## Simulator-only components

The following components are useful test and integration ports but provide no
airborne assurance or operational credit:

| Component | Boundary role | Limitation |
|---|---|---|
| Gazebo and deterministic simulation adapters | Supply test state and images | Simulated truth has no sensor-error, installation, or integrity claim unless a test explicitly injects and measures it |
| Aviate SITL and USB CDC development links | Exercise a vehicle-source adapter | Link format and successful parsing do not qualify a sensor, estimator, cable, power source, or installation |
| Session host and WebTransport | Forward telemetry and media | Receipt time is not source time; forwarding does not refresh age or integrity |
| Browser and JavaScript bridge | Drive the display core and test operator interaction | Browser scheduling, lifecycle, fonts, memory, and DOM/Canvas behavior are not a qualified display platform |
| WebAssembly instrument module | Exercises portable core behavior | A build target alone provides no tool, platform, or airborne software approval |
| Canvas renderer and browser display | Render simulator scene commands | It is nondeterministic across browser/platform combinations and is always **SIM / NOT FOR FLIGHT** |
| Instrument slider harness and replay | Inject boundary and failure cases | Injected values are test data and test mode remains visibly identified |

An aircraft project may retain the portable pure-state and scene contracts only
after allocating their intended functions and verification evidence. Sensor/bus
adapters, platform services, renderer, display hardware, independent monitor,
power, installation, and continued-airworthiness data require their own safety
and assurance treatment. This document does not allocate DALs.

## SVS, TAWS, and compositor containment

SVS is an optional removable background under
[`AIR-OUT-005`](requirements.md#air-out-005). It consumes database and navigation
evidence, but it neither supplies nor validates primary attitude/air data. The
critical two-dimensional symbology path must remain available when the SVS
model, database, terrain renderer, or GPU fails. The compositor enforces the
priority `SVS background < primary symbology < warning/caution`.

TAWS logic is outside this boundary. If a later display consumes TAWS alerts,
they arrive as independent monitored inputs under
[`AIR-IN-012`](requirements.md#air-in-012) and remain visually and functionally
distinct from terrain shading. This preserves
[`AIR-OUT-010`](requirements.md#air-out-010): **SVS is not TAWS**.

## HUD-SIM boundary

HUD-SIM ends at a calibrated simulator projection under
[`AIR-OUT-006`](requirements.md#air-out-006). It may claim conformality only for
the declared fixed design eye, camera, lens/intrinsics, camera-to-body
extrinsics, field of view, boresight calibration, video capture time, display
geometry, and latency budget. Invalid or absent evidence invokes
[`AIR-UNAV-006`](requirements.md#air-unav-006).

Without those inputs, the output is a non-conformal repeater under
[`AIR-OUT-007`](requirements.md#air-out-007), visibly marked **NON-CONFORMAL /
NOT A HUD**. Optical collimation, combiner, head motion/eyebox, luminance,
alignment, installation, maintenance, and operational credit are excluded by
[`AIR-OUT-008`](requirements.md#air-out-008).

## Safety and review boundary

This baseline defines inputs to future functional hazard assessment; it does not
perform that assessment or select an assurance level. Aircraft-level functions,
failure conditions, independence, common causes, crew procedures, dispatch
configuration, and certification basis must be selected before assurance is
allocated. Closure requires the independent review record specified by
[`AIR-BAS-006`](requirements.md#air-bas-006).
