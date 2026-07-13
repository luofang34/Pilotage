# ADR-0021: Versioned, hashed simulator camera/design-eye calibration

- Status: Accepted
- Date: 2026-07-12

## Context

HUD-01 (ADR-0020) made a video frame's capture identity and its clock mapping
to the aircraft state traceable, and left the conformal gate closed until a
*recognized calibration* is published — because a conformal overlay projects
world features onto the image through the camera's intrinsics, distortion, and
mounting, and drawing through a wrong or unversioned calibration would register
symbology in the wrong place. Nothing yet published a calibration, so live
conformal could never open.

A calibration must therefore be a first-class, versioned, verifiable artifact:
every unit and coordinate frame explicit, a stable identity, a content hash so
corruption or silent edits are caught, an effective window, provenance, and the
residual error of the fit. It must also come from *one* source — a scattered
default FOV, principal point, or set of extrinsics is exactly the kind of silent
assumption that puts symbology in the wrong place.

**This is SIM.** The simulator has a synthetic pinhole camera and a design-eye
reference; it has none of the optics a real head-up display qualification turns
on — combiner, collimation, eyebox, installation alignment. Conflating the two
would be a safety-relevant category error.

## Decision

- **A calibration is a `CameraCalibration`**: a [`CameraGeometry`] (pinhole
  intrinsics, Brown-Conrady distortion, viewport, field of view, body-to-camera
  extrinsics, design eye, boresight) plus a [`CalibrationIdentity`] (id, content
  version, tool version, effective window, provenance, residual RMS/max,
  validity status). It lives in `pilotage-adapter-api::calibration`.

- **Every unit and frame is explicit.** Field names carry units (`_px`, `_m`,
  `_rad`); the extrinsics name both frames using the `pilotage-frames`
  vocabulary (`FrameId::Body` → `FrameId::Installation`, the sensor mount) and
  never invent a frame; the optical convention (OpenCV: `+Z` forward, `+X`
  right, `+Y` down) is a stored enum, not an assumption; a units marker is
  serialized so a future unit change is a visible schema event.

- **Content hash, verifier recomputes** (the REN-02 glyph-pack pattern). A fixed
  little-endian canonical byte form is hashed with SHA-256; the recorded hash is
  stored beside the artifact, never inside its canonical bytes. `verify`
  recomputes and fails closed on a mismatch, a non-`Valid` status, or an
  out-of-window time; a separate camera check fails closed when the frame came
  from a different camera than the calibration describes. `f64` fields are their
  IEEE-754 bytes, so the hash is bit-exact across the Rust producer and the
  browser verifier.

- **Derivable quantities are not stored (schema v2).** A value that can be
  computed from other stored fields is never serialized, because a stored copy
  can be made to disagree with the fields it should follow. The **field of view**
  is derived from the viewport and focal lengths
  (`CameraGeometry::field_of_view`); the alignment budget's **pixel→angle factor
  and totals** are derived from the stored allowances and the focal lengths
  (`derive_budget`). Only irreducible inputs live in the canonical bytes. This
  removes an entire class of "consistent hash, inconsistent geometry" lie a
  validator would otherwise have to chase.

- **Hash integrity is not semantic validity.** A hash-consistent artifact can
  still carry a NaN focal length, a zero viewport, a non-positive focal, a
  principal point outside the image, extrinsics naming the wrong frames, a
  non-unit rotation or boresight, an inverted effective window, negative
  residuals, a **non-positive declared allowance**, or an **intrinsic budget
  that fails to cover the measured recovery residual**. `calibration::validate`
  checks every such invariant and fails closed with a distinct typed reason,
  never clamping or repairing, and `verify` runs it after the hash check. The
  browser admission path (`clients/web/calibration.js`) parses the base fields
  and **independently** re-validates the same invariants and re-derives the few
  numbers it surfaces; Rust is the reference validator and the browser is a
  deliberately thin, subordinate check. Each invariant class has a
  fault-injection test where the bytes and their recorded hash agree yet the
  data is invalid.

- **The calibration publishes its contribution to the conformal alignment error
  budget**, hashed as its stored **allowance components** and surfaced through
  the browser admission as one **derived** angular bound with provenance. It is a
  conservative worst-case (linear, not root-sum-square) sum of: the *recovered*
  intrinsic residual budget converted to an angle via the derived pixel→angle
  factor (`1 / min(focal_x, focal_y)`), plus *declared, not recovered*
  engineering allowances for the distortion/model assumption, the extrinsics
  rotation, the boresight, and the design-eye parallax at a reference range. Each
  declared allowance carries a stated rationale, is validated **strictly
  positive** (never zero — "declared exactly" is still an assumption to bound),
  and the intrinsic budget is validated to **cover** the measured recovery
  residual so the calibration cannot understate its own fit error. The next HUD
  increment composes the single derived number into its total budget.

- **Registry conflicts fail closed.** When two calibrations share an id but
  differ in content (a different hash — e.g. a bumped version), the browser
  registry admits **neither**: the id is marked conflicting and never resolves,
  so a consumer never picks an arbitrary winner. Exact duplicates (same id, same
  hash) are deduped, not conflicts.

- **One deterministic simulator tool** (`calibration::recovery`) generates a
  fixed grid of synthetic targets, projects them through a known pinhole model,
  quantizes to whole pixels (the only error, so the run is deterministic with no
  wall-clock flakiness), recovers the intrinsics by linear least squares, and
  reports the residuals and the recovered-vs-known errors. Documented
  tolerances: focal length within 1%, principal point within 1 px, residual RMS
  under 0.5 px. A CI test asserts the published calibration's fit is within
  these. Extrinsics and distortion are *declared from the sim world*, not
  recovered, and the artifact says so.

- **One source of calibration.** The single published artifact
  (`calibration::sim::sim_fpv_calibration`) is pinned to the Gazebo world's
  onboard camera (`sim/worlds/pilotage_yard.sdf`: 320×240, 1.396 rad horizontal
  FOV, body mount `(1.1, 0, 0.3) m`). There is no default FOV, center, or
  extrinsics anywhere in the conformal path; the video path carried none to
  begin with. The Gazebo adapter stamps the onboard camera's frames with this
  calibration id; a camera with no published calibration stamps `NONE`.

- **The browser feeds its recognized set from the hash-verified artifact.**
  `clients/web/calibration.js` loads the published artifact (canonical bytes +
  recorded hash), recomputes the SHA-256, parses only the header, and admits a
  calibration id for a frame's camera only when the hash matches, the status is
  `Valid`, the time is within the window, and the camera matches. A missing,
  mismatched, corrupt, expired, or wrong-camera calibration yields no recognized
  id, so the HUD-01 conformal gate stays closed. The calibration
  effective-window check uses genuine wall-clock time (a calibration is valid
  for a real-time period), unlike the capture-time association.

- **SIM, never optical HUD qualification.** No artifact, field, type, or
  document here may be read as HUD airworthiness or optical qualification. The
  simulated camera and synthetic design eye are explicitly *not* a combiner,
  collimation, eyebox, or installation alignment. SIM / NOT FOR FLIGHT.

## Consequences

- Live, the FPV calibration now resolves and the calibration side of the gate
  can open. The gate as a whole still stays closed live, honestly: the Gazebo
  rover publishes planar pose, not an avionics snapshot to associate against,
  and Aviate's video-to-flight clock mapping is unavailable. No overlay is
  drawn; the full open path is exercised by tests with synthetic data.
- A future real camera or a re-calibration is a new artifact with a new hash and
  a bumped version, verified by the same recompute-and-compare path.
- The `f64`-bit canonical form couples the Rust and browser layouts; a change to
  either must bump `CALIBRATION_SCHEMA_VERSION` and re-record the hash.
