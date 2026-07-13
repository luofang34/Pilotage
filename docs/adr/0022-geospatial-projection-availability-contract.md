# ADR-0022: Transport-independent geospatial, projection, and availability contract

- Status: Accepted
- Date: 2026-07-13

## Context

Synthetic vision (SVS) and a conformal head-up display both need to place world
geometry in an image: a runway where the runway is, terrain where the terrain
is. Getting that right depends on agreeing, before any pixels, on what a
position *means* (which datum, which vertical reference), which clock a reading
came from, how trustworthy it is, how the camera projects, and — crucially —
when the scene must **not** be drawn because an input is missing or untrusted.
Those agreements are a contract, and they must be independent of the transport
(WebTransport today, something else tomorrow) and of any particular renderer.

The failure mode this contract exists to prevent is a *plausible wrong scene*: a
height read against the wrong datum, an age computed across two clocks, a
longitude that means two places at the anti-meridian, or a terrain database gap
papered over as normal terrain. Each of those is a silent inference, and each
would put symbology in the wrong place while looking correct.

This is a SIM/engineering contract; completion is **not** SVS or SVGS approval.
SIM / NOT FOR FLIGHT.

## Decision

- **A new foundational crate, `pilotage-geo`**, holds the typed contract. It is
  `#![no_std]`, allocation-free, and `forbid(unsafe_code)`, so it compiles for a
  bare-metal target and can sit beneath both consumers.

- **Datum discipline; no bare altitude.** A height is a [`VerticalPosition`] that
  always carries a [`VerticalDatum`] (ellipsoid, MSL, AGL, barometric-indicated,
  pressure, or local-relative), a geoid model for MSL, and a local origin for a
  relative height. A [`GeodeticPosition`] always carries a [`HorizontalDatum`].
  Unknown datums are refused. The vertical vocabulary is minted here — not taken
  from instrument-state's `AltitudeClass` — because this crate is foundational
  and instrument-state is a consumer; the module documents the field-for-field
  mapping so the two never drift.

- **Longitude is normalized; poles and seams are explicit.** `new` wraps
  longitude to `[-180, 180)`, so the anti-meridian has one canonical spelling;
  `at_pole` and `on_antimeridian` flag the degenerate cases; and `tile` floors
  into exactly one tile so a seam never oscillates.

- **Identity and coherence reuse the AV-01 `MeasurementStamp` shape.** A
  [`SourceStamp`] carries source id, incarnation, generation, sequence,
  acquisition [`Epoch`] (clock **and** scale **and** nanos), integrity,
  accuracy, and a coherent-snapshot id. Age is only computed between readings on
  the same clock and scale — across clocks it is `None`, never a silently
  inferred difference — and coherence requires a declared, matching snapshot id.

- **The view derives its field of view.** [`ProjectionView`] stores the
  viewport, focal lengths, projection, near/far policy, minification,
  convention, and camera pose (frames named `Body` → `Installation`); the field
  of view is **derived** from viewport and focal (aligning with the ADR-0021
  calibration contract), never stored.

- **Availability is finite, deterministic, and traceable.** [`SvsAvailability`]
  is `Available`, `Degraded(reason)`, or `Unavailable(reason)` over a finite
  [`AvailabilityReason`] set (position, attitude, integrity, time/coherence,
  calibration, database, coverage, renderer). `assess` maps the stated health of
  each input to a verdict by a **fixed precedence**, so the same inputs always
  yield the same verdict and reason. Health is stated, never defaulted: an
  unknown input is `Failed`, and "nothing known" is `Unavailable`, never a
  normal scene.

- **TAWS is an independent input.** A [`TawsAlert`] is a separate type with its
  own source stamp; nothing derives a TAWS alert from the SVS scene or folds SVS
  availability into a terrain hazard. The availability API's signature contains
  no TAWS type, so the two cannot leak into one another.

- **The ABI is versioned, fixed-size, and fail-closed.** One [`SvsFrame`]
  encodes to a fixed little-endian byte block led by a version `u32`;
  [`decode_frame`] refuses a truncated buffer, an unsupported version, an
  enumerated value outside its known set (reporting the actual value), a
  non-finite coordinate, and a semantically malformed block (e.g. an MSL height
  with no geoid). Encoding is allocation-free (it writes a fixed array).

## Consequences

- Datum, units, reference, and clock domain cannot be silently inferred: they
  are type-level, and decode fails closed on anything unknown.
- A missing, inconsistent, or untrusted input resolves to degraded/unavailable
  with a traceable reason, never a plausible normal scene.
- A renderer and a transport can be swapped without touching the contract; the
  frame ABI is their only coupling and it is versioned.
- The vertical-datum vocabulary duplicates instrument-state's `AltitudeClass`
  intentionally (dependency direction), pinned by the documented mapping table;
  if either changes, the mapping is the place they are reconciled.
