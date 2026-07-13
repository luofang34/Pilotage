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

- **Datum discipline; no bare altitude, no implicit realization.** A height is a
  [`VerticalPosition`] that always carries a [`VerticalDatum`] (ellipsoid, MSL,
  AGL, barometric-indicated, pressure, or local-relative) **and the identity that
  datum needs**: a declared geoid model for MSL, a terrain/ground reference for
  AGL, an applied altimeter-setting identity for barometric-indicated, and a
  local origin for a relative height. A [`GeodeticPosition`] always carries a
  [`HorizontalDatum`] and, for a realization-bearing datum (NAD83, ITRF), a
  declared realization/reference-epoch id. Unknown datums, undeclared
  realizations, and missing identities are refused with a specific typed reason
  (a horizontal fault never reports a vertical-datum reason), and an invalid tile
  size is a `Result`, never a plausible zero tile. The vertical vocabulary is
  minted here — not taken from instrument-state's `AltitudeClass` — because this
  crate is foundational and instrument-state is a consumer; the module documents
  the field-for-field mapping so the two never drift.

- **Longitude is normalized; poles and seams are explicit.** `new` wraps
  longitude to `[-180, 180)`, so the anti-meridian has one canonical spelling;
  `at_pole` and `on_antimeridian` flag the degenerate cases; and `tile` floors
  into exactly one tile so a seam never oscillates.

- **Identity is separate from function-specific quality.** A [`SourceStamp`]
  carries only source id, incarnation, generation, sequence, acquisition
  [`Epoch`] (clock **and** scale **and** nanos), integrity, and a
  coherent-snapshot identity — never an accuracy. Position accuracy is a length
  (`PositionQuality`, millimeters) and attitude accuracy is an angle
  (`AttitudeQuality`, milliradians): distinct types, so a position accuracy can
  never be read as an attitude's. Age fails closed — a different clock, a
  different scale, or a future sample is a typed `AgeError`, never a saturated
  zero — and coherence binds the full snapshot identity (producer incarnation,
  generation, instance id) and time base, so an equal numeric id from a
  different stream is not coherent.

- **The view references the one validated calibration.** There is exactly one
  authoritative camera model — the versioned, hashed calibration artifact
  (ADR-0021). [`ProjectionView`] does not re-mint intrinsics, distortion,
  viewport, pose, field of view, or the alignment bound; it *references* the
  accepted calibration by id and content hash **only**, and adds only the
  render-time policy (projection kind, near/far, minification). The alignment
  bound and geometry come from *resolving* that reference against a verified
  artifact (in the `std` calibration contract), so a producer cannot write an
  understated bound onto the wire — there is no bound field to write. Perspective
  and orthographic are typed payloads — a focal-derived field of view is not an
  orthographic invariant.

- **One canonical `CalibrationId` in a shared leaf.** The calibration identity is
  a single type, [`CalibrationId`], owned by a dependency-free `no_std` leaf crate
  (`pilotage-calibration-id`) that both `pilotage-geo` and the `std` calibration
  contract (`pilotage-adapter-api`) depend on and re-export. Neither owns the id
  relative to the other, and adapter-api pulls only the one-`u32` leaf — not the
  whole geospatial/ABI contract — to name it. A synthetic-vision projection
  reference and the calibration artifact it points at are therefore the **same**
  type over one identity space, not two mirrored `u32`s a lossy conversion could
  drift apart. Uniqueness is enforced two ways: a compile-time coercion in each
  re-exporting crate proves its `CalibrationId` is the leaf's, and a repository
  guard (in `check-structure.sh`, run in CI) fails the build if a second
  `struct CalibrationId` is defined anywhere in the tree. Geometry, lifecycle,
  content hash, and the alignment budget stay authoritative in the calibration
  artifact; geo's [`CalibrationRef`] carries only the shared id plus the
  artifact's content hash. (The vertical-datum vocabulary is still *mirror*ed
  rather than shared — that mirror exists to avoid a dependency on a consumer,
  `instrument-state`; the calibration id has no such direction problem, so it is
  shared outright.)

- **Availability is derived, never self-reported.** The wire carries **no**
  availability. A frame decodes to a [`ValidatedSvsFrame`] whose
  [`SvsAvailability`] is *computed* from the validated inputs: position and
  attitude health from the **worse** of their integrity and their accuracy
  (position 1-sigma in millimeters, attitude 1-sigma in milliradians, against
  degrade/fail limits), time/coherence from the age, future-sample check,
  and coherent-snapshot binding against the frame reference time; only the
  inputs the contract cannot check (navigation-integrity monitor, calibration,
  database, coverage, renderer) are producer-stated. `assess` maps input health
  to `Available`, `Degraded(reason)`, or `Unavailable(reason)` over a finite
  [`AvailabilityReason`] set by a **fixed precedence**. An untrusted, grossly
  inaccurate, or incoherent input can never yield an available scene, and there
  is no wire byte a producer could set to claim otherwise.

- **The freshness and accuracy limits are an explicit [`AvailabilityProfile`],
  chosen by the receiver, off the wire.** The degrade/fail limits are not baked
  into the derivation: they live in an [`AvailabilityProfile`] (identity, version,
  and four validated limit pairs — freshness age, usable age, position accuracy,
  attitude accuracy) that the caller passes at the validation/decode boundary
  (`RawSvsFrame::validate(&profile)`, `decode_frame(buf, &profile)`). The fields
  are **private**: the only ways to obtain a profile are the checked
  [`AvailabilityProfile::new`] and the controlled [`AvailabilityProfile::simulator`],
  so a struct literal cannot skip the check (a `compile_fail` doctest pins that a
  direct construction does not compile). `new` rejects a zero or non-monotonic
  pair (a *fresh* limit must be strictly tighter than its *usable* limit) with a
  typed [`GeoError::InvalidAvailabilityProfile`], so a profile can never admit a
  reading it should reject. There is deliberately **no** `Default` and no free
  function that picks a profile: SIM limits are never presented as operational
  limits by omission, and the SIM constants are not a public surface — the profile
  and its accessors are. The current SIM allocation is one named profile,
  [`AvailabilityProfile::simulator`] (its limits pinned by independent golden
  literals, not by the numbers it is built from); a flight or orbital function
  would be a different named profile with its own limits. The selected profile's identity is
  bound into the [`ValidatedSvsFrame`] (`profile()`), so a verdict is traceable to
  the exact limits that produced it.

  **The profile is receiver-controlled evaluation context, not producer data, so
  it stays off the wire and ABI v2 is unchanged.** The bytes are what the producer
  measured; the limits are the receiver's intended-function policy, and the same
  block can legitimately be judged available under one function and unavailable
  under a stricter one. Encoding never serialized availability and still does not
  serialize the profile: `encode_frame` takes no profile, the byte layout and
  `ABI_VERSION` are untouched, and decoding one block under two profiles yields
  the identical parsed position/attitude/view with only the derived verdict (and
  the bound profile identity) differing. The alternative — stamping a profile id
  onto the wire — was rejected: it would let a producer's declared policy override
  a receiver's, which inverts who is allowed to decide a scene is safe to draw.

- **TAWS is an independent input.** A [`TawsAlert`] is a separate type with its
  own source stamp; nothing derives a TAWS alert from the SVS scene or folds SVS
  availability into a terrain hazard. The availability API's signature contains
  no TAWS type, so the two cannot leak into one another.

- **The ABI is versioned, fixed-size, and fail-closed.** One frame encodes to a
  fixed little-endian byte block led by a version `u32`; the length must match
  exactly (trailing bytes are as suspect as truncation). [`decode_frame`] takes
  the receiver's evaluation profile (which drives only the derived verdict, never
  how the bytes are parsed) and refuses a wrong length, an unsupported version,
  an enumerated value outside its known set (reporting the actual value), a
  non-finite coordinate, a non-unit aircraft attitude, an incomplete datum
  identity (an MSL height with no geoid, an AGL height with no terrain reference,
  a barometric-indicated height with no applied setting, a NAD83/ITRF datum with
  no realization), and an incomplete calibration reference. Only a
  [`ValidatedSvsFrame`] can be encoded, so an invalid frame cannot be serialized;
  encoding is allocation-free.

## Consequences

- Datum, units, reference, and clock domain cannot be silently inferred: they
  are type-level, and decode fails closed on anything unknown.
- A missing, inconsistent, or untrusted input resolves to degraded/unavailable
  with a traceable reason, never a plausible normal scene.
- A renderer and a transport can be swapped without touching the contract; the
  frame ABI is their only coupling and it is versioned.
- The same wire frame can be evaluated under different intended-function limits
  by choosing a different [`AvailabilityProfile`] at the receiver; SIM limits are
  one named profile, never a silent default, and the profile the verdict was
  judged against travels with the verdict. Because the profile is off the wire,
  ABI v2 bytes are unchanged.
- There is one `CalibrationId` in the program, owned by a shared `no_std` leaf
  crate and re-exported by both geo and adapter-api, so a projection reference and
  a calibration artifact cannot drift into two identity spaces and adapter-api does
  not pull the geospatial contract to name a `u32`; per-crate compile-time
  coercions and a repository guard fail the build if a second definition appears.
- The vertical-datum vocabulary duplicates instrument-state's `AltitudeClass`
  intentionally (dependency direction), pinned by the documented mapping table;
  if either changes, the mapping is the place they are reconciled.
