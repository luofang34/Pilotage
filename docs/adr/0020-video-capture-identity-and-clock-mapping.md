# ADR-0020: Video frames carry a capture identity and an explicit clock mapping

- Status: Accepted
- Date: 2026-07-12

## Context

A conformal overlay draws symbology registered to the world as seen in a camera
image: a runway outline on the runway, a flight-path marker where the aircraft
is actually going. That registration is only correct if the overlay is computed
from the aircraft state **at the instant the image was captured**, not the
instant the browser happened to receive and decode the frame. Network jitter,
host queueing, and JPEG decode latency put tens to hundreds of milliseconds
between capture and display; at approach speeds that is meters of registration
error.

The media plane as it stood could not support this even in principle. A video
frame on the wire (ADR-0016) was `[0x02][source_id][fourcc][u32 len][payload]`:
it carried no capture time, no source identity beyond the routing byte, and no
sequence. The Gazebo adapter's `RawVideoFrame` did carry a sidecar sim-time
capture tick, but the host discarded it at the encode step. So a consumer had
nothing to correlate a frame against a telemetry sample, and nothing to detect a
duplicated or reordered frame — a replayed frame would blit over a newer one and
its age would silently reset.

Two distinct clocks are also in play and must not be conflated. A frame is
captured on the camera/sim clock; the aircraft state is estimated on the flight
controller's clock (ADR-0009's domain discipline, ADR-0018's `MeasurementStamp`).
Whether those two clocks can be related at all — and with what error — is a
property of the integration, not something a consumer may assume.

## Decision

- Every video frame carries a **capture identity** and an explicit
  **clock mapping**, defined once in `pilotage-adapter-api` and preserved
  unchanged from the adapter to the browser.

- The capture identity **reuses the AV-01 `MeasurementStamp` vocabulary**
  (ADR-0018) rather than inventing a parallel one: a captured frame is a
  measurement group whose acquisition clock is the camera's. It carries the
  routing source id, an opaque per-attachment incarnation, a source epoch, a
  wrapping `u32` frame sequence, the capture time in nanoseconds, and the
  capture clock domain. Alongside it travel a camera id and a calibration id
  (`0` = no calibration published), and the clock mapping.

- The clock mapping is `CaptureClockMapping`, whose **default is `Unavailable`**.
  A mapping is asserted only when an adapter can actually establish one, as
  `Bounded { target, offset_ns, error_bound_ns }`: the flight-state clock the
  capture time maps into, the signed offset, and the **quantified error bound** a
  consumer budgets against. The Gazebo sidecar stamps both its frames and its
  telemetry from one sim clock, so it declares the identity mapping (offset 0,
  error 0). Aviate captures on the sim sidecar clock but estimates flight state
  on the vehicle-boot clock with no correlation between them, so it declares
  `Unavailable` — honestly, rather than fabricating a stamp.

- The wire format is **explicitly versioned**. A new stream-kind byte `0x03`
  precedes a v2 body: a fixed capture-identity header, then the existing
  `[fourcc][u32 len][payload]`. `0x02` keeps its exact prior meaning; a reader
  that does not recognize `0x03` skips the stream, exactly as it skips an unknown
  FourCC, so a v2 host degrades gracefully against an older client. The host
  additionally stamps host **receive** and **publication** times into the header,
  kept distinct from the capture time so a consumer never conflates host receipt
  with acquisition — the receipt time this whole ADR exists to stop relying on.

- The browser runs a **per-source identity tracker** mirroring the avionics
  ingestion discipline (`telemetry-ingress.js`) and reusing its serial-number
  comparison: freshness advances only on a strictly newer epoch/sequence. A
  duplicate, reordered, stale-epoch, or wrong-camera frame is dropped and leaves
  the accepted state untouched, so a replayed frame can neither displace a newer
  frame nor refresh its age. An epoch reset, a new incarnation, or a
  **calibration-ID change** is accepted as an explicit discontinuity — the
  conformal timeline never silently continues across a change of camera model.

- The browser exposes a **conformal gate**, `conformalGate(meta,
  candidateSnapshotIdentity)`, that **fails closed** and consumes BOTH the frame
  metadata AND the candidate aircraft snapshot's identity (an AV-01
  `MeasurementStamp`; its `clock` is read here). "Bounded" is not sufficient. A
  frame is conformal-ready only when all hold:
  - the clock mapping is available, and its **target clock matches** the clock
    the candidate snapshot is expressed in;
  - the mapping's quantified error is within a **configured budget**
    (`DEFAULT_MAX_CLOCK_ERROR_NANOS`, a named constant with a stated rationale);
  - applying the mapping's signed offset to the capture time does **not overflow
    or underflow** the `u64` nanosecond range (it refuses rather than wrapping);
  - the frame's **calibration ID is published and recognized**
    (`CalibrationId::NONE` / zero, or an unrecognized id, keeps the gate closed).

  The gate reports `mappingValid` (the clock side) separately from
  `conformalReady` (which additionally requires calibration). This is a SIM
  prerequisite for a HUD-SIM overlay, NOT an airborne HUD capability, and is NOT
  FOR FLIGHT.

- The browser performs **capture-to-snapshot association**
  (`snapshot-association.js`). It observes the aircraft snapshots the AV-01
  ingestion has already accepted — it does not gate them — into a bounded
  history ring, each entry carrying the snapshot's `MeasurementStamp` identity.
  Given an accepted video frame, it maps the capture time through the frame's
  mapping into the snapshot clock domain and selects the **nearest snapshot by
  acquisition time**. The result is a typed verdict carrying the associated
  snapshot identity, the mapped capture time, and a **total error** (the
  mapping's error bound plus the association delta). It fails closed: an empty
  history, a clock-domain mismatch, a nearest snapshot from a superseded source
  incarnation, or a total error over budget all yield "not ready". Association
  runs only on frames the identity tracker accepted (so a replay never
  associates fresh), and the verdict is finally passed through `conformalGate`,
  so the calibration/clock/mapping checks can never be bypassed. The browser
  never associates against the *latest* snapshot by receipt — only by capture
  time — which is the whole point of this ADR.

## Consequences

- A future conformal overlay can select the aircraft state corresponding to a
  frame's capture time and know the residual clock error, or refuse to draw when
  no mapping is available — the decision is data-driven, not implicit.
- Adapters that genuinely lack a capture-clock correlation report that fact; the
  system never invents a mapping to make an overlay appear admissible.
- Adding the header costs a fixed number of bytes per frame and one serial
  comparison per frame in the browser; both are negligible against the JPEG.
- The v1 `0x02` framing remains defined for compatibility and is exercised only
  by the framing round-trip tests; the host emits `0x03`.
