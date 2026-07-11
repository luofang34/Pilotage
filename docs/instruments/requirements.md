# Instrument intended-function requirements

This registry assigns stable identifiers to the inputs, outputs, modes, flags,
unavailable conditions, operating assumptions, and process constraints in the
instrument boundary. “Shall” marks a requirement; it does not claim that the
implementation already satisfies it.

References use links such as
[`AIR-BAS-001`](requirements.md#air-bas-001). Identifiers are never reused.

## Basis and governance

<a id="air-bas-001"></a>
### AIR-BAS-001 — Simulation identification

Every browser, WebAssembly, Canvas, WebTransport, Gazebo, and test-harness
instrument surface shall continuously and conspicuously identify itself as
**SIM / NOT FOR FLIGHT**.

<a id="air-bas-002"></a>
### AIR-BAS-002 — Reference architecture without approval claim

The architecture shall use conservative dual-pilot Part 25 IFR primary-flight-
information behavior as a design reference while recording the target aircraft,
certification authority, certification basis, and installed equipment as
unselected; the reference shall not be represented as approval or operational
credit.

<a id="air-bas-003"></a>
### AIR-BAS-003 — Hazard-derived assurance

Development assurance shall be allocated per function and item only after the
applicable aircraft-level hazard assessment; no single DAL shall be assigned to
the complete Pilotage application by this baseline.

<a id="air-bas-004"></a>
### AIR-BAS-004 — Explicit reference metadata

No display function shall infer an unspecified unit, coordinate frame, altitude
datum, heading reference, time domain, source identity, integrity level, or
camera calibration; missing required metadata shall drive a declared unavailable
condition.

<a id="air-bas-005"></a>
### AIR-BAS-005 — Feature traceability

Every new or changed display feature shall cite at least one intended-function
requirement in its issue and pull request, and changes in intended behavior shall
update this baseline before implementation acceptance.

<a id="air-bas-006"></a>
### AIR-BAS-006 — Required independent reviews

The intended-function baseline shall not be approved or its issue closed until
the project owner and qualified safety and human-factors reviewers have recorded
their review disposition in the version-controlled review record.

<a id="air-bas-007"></a>
### AIR-BAS-007 — Deterministic presentation

An identical coherent input snapshot, display configuration, and renderer
version shall produce the same display state, including flags, reversion, layer
priority, and command ordering.

## Inputs

<a id="air-in-001"></a>
### AIR-IN-001 — Attitude input

Attitude input shall contain a finite normalized orientation, body angular
rates, declared body and navigation frames, source identity, source time,
sequence or equivalent ordering evidence, quality, integrity, and accuracy.

<a id="air-in-002"></a>
### AIR-IN-002 — Position and altitude input

Position and altitude input shall declare coordinate frame, horizontal datum,
altitude type and datum, units, source identity, source time, quality, integrity,
and accuracy; local NED height shall not be labelled barometric altitude.

<a id="air-in-003"></a>
### AIR-IN-003 — Velocity input

Velocity input shall declare axes, reference frame, air-relative or ground-
relative meaning, units, source identity, source time, quality, integrity, and
accuracy.

<a id="air-in-004"></a>
### AIR-IN-004 — Air-data input

Indicated airspeed, pressure altitude, barometric correction, vertical speed,
and related air-data values shall each identify their measurement or derivation,
datum, units, source identity, source time, quality, integrity, and accuracy.

<a id="air-in-005"></a>
### AIR-IN-005 — Heading and navigation input

Heading, track, course, bearing, deviations, distance, and vertical guidance
shall declare true or magnetic reference, magnetic variation source where
applicable, navigation source and mode, units, source time, quality, integrity,
and accuracy.

<a id="air-in-006"></a>
### AIR-IN-006 — Wind input

Wind direction and speed shall declare “from” or “to” convention, true or
magnetic reference, reference frame, units, source identity, source time,
quality, integrity, and accuracy.

<a id="air-in-007"></a>
### AIR-IN-007 — Crew selections and aircraft configuration

Selected altitude, heading, course, barometric correction, V-speeds, display
mode, and airframe display profile shall identify source, authority, units,
range, configuration revision, and validity.

<a id="air-in-008"></a>
### AIR-IN-008 — Time and ordering evidence

Each independently updated input group shall carry source identity, source epoch,
monotonic sequence, source or capture time, receive time, and a declared mapping
between time domains sufficient to detect replay, reordering, reset, and age.

<a id="air-in-009"></a>
### AIR-IN-009 — Per-signal validity evidence

Quality, integrity, accuracy, freshness, range, and fault reason shall be carried
per independently sourced signal; a global quality value shall not make a signal
valid when its own evidence is absent or failed.

<a id="air-in-010"></a>
### AIR-IN-010 — Video and conformal calibration input

Video used for conformal simulation shall provide capture time, image dimensions,
lens model and intrinsics, camera-to-body extrinsics, design eye, field of view,
boresight calibration, calibration revision, and validity.

<a id="air-in-011"></a>
### AIR-IN-011 — Synthetic-vision database input

Synthetic-vision terrain, obstacle, runway, taxiway, and airport data shall
identify datum, coverage, resolution, effective period, provenance, database
revision, integrity result, and load status; absent evidence shall make the
affected synthetic-vision content unavailable.

<a id="air-in-012"></a>
### AIR-IN-012 — Independent alert input

Alerts supplied by TAWS, traffic, weather, flight-guidance, or other aircraft
systems shall identify the originating function, alert identity, priority,
latch/acknowledgement state, source time, integrity, and availability; the
display shall not derive an undeclared alert from synthetic-vision graphics.

<a id="air-in-013"></a>
### AIR-IN-013 — Renderer health input

The display function shall receive independent evidence of renderer progress,
frame generation, command-buffer integrity, backend status, and output-path
health sufficient to detect a retained last-good image.

## Outputs and functions

<a id="air-out-001"></a>
### AIR-OUT-001 — Primary flight display surface

The PFD function shall consume validated attitude, air data, altitude, heading,
vertical state, crew selections, modes, and alert state and shall output a
prioritized two-dimensional flight display with an explicit status for every
required input; browser output remains simulation-only and is not approved PFI.

<a id="air-out-002"></a>
### AIR-OUT-002 — Horizontal situation display surface

The HSI function shall consume validated heading, track, navigation source,
course, deviation, distance, wind, selections, and alert state and shall output
their reference and navigation mode or an explicit unavailable presentation.

<a id="air-out-003"></a>
### AIR-OUT-003 — Conventional instrument surface

The conventional-instrument function shall consume the same validated state as
the integrated display and shall output independently identifiable attitude,
airspeed, altitude, heading, turn/slip, and vertical-speed indications without
fabricating a missing source.

<a id="air-out-004"></a>
### AIR-OUT-004 — Failure and miscompare presentation

Every credited display output shall present stale, missing, failed, invalid,
miscompared, out-of-range, and renderer-failure states conspicuously and shall
not silently retain or substitute a last-good value.

<a id="air-out-005"></a>
### AIR-OUT-005 — Synthetic-vision background

SVS may output registered terrain, obstacle, airport, runway, and pathway
graphics beneath primary symbology only when every required database, position,
attitude, time, and integrity input is available; it supplies supplemental
situation awareness unless a separately approved intended function grants
additional credit.

<a id="air-out-006"></a>
### AIR-OUT-006 — HUD-SIM conformal output

HUD-SIM may output simulation-only conformal symbology from validated flight
state, video timing, camera calibration, design eye, and projection geometry;
it shall remain labelled **HUD-SIM / NOT FOR FLIGHT** and conveys no airborne
HUD operational credit.

<a id="air-out-007"></a>
### AIR-OUT-007 — Non-conformal repeater output

A display lacking valid design-eye and camera/optical calibration may output a
simulation repeater, but it shall be labelled **NON-CONFORMAL / NOT A HUD** and
shall not display conformal flight-path, horizon, runway, or guidance claims.

<a id="air-out-008"></a>
### AIR-OUT-008 — Airborne optical HUD exclusion

An installed airborne HUD, head-worn display, combiner, optical collimation,
eyebox, luminance, alignment, installation, and continued-airworthiness function
is outside this system boundary and shall not be claimed by HUD-SIM output.

<a id="air-out-009"></a>
### AIR-OUT-009 — SVGS exclusion

This baseline supplies no Synthetic Vision Guidance System function, low-
visibility operational credit, or synthetic guidance for landing; adding SVGS
requires a distinct intended function, safety assessment, applicable performance
standard, guidance source, monitoring, failure presentation, and approval basis.

<a id="air-out-010"></a>
### AIR-OUT-010 — TAWS independence

SVS shall not be described as TAWS and shall not suppress, replace, generate, or
claim the terrain-alerting function; a future TAWS interface remains an
independent monitored input whose alerts have priority over SVS graphics.

## Modes

<a id="air-mode-001"></a>
### AIR-MODE-001 — Conventional horizon mode

Conventional horizon mode shall present a deterministic sky/ground attitude
background and required primary symbology without dependence on terrain,
obstacle, airport, video, or camera-calibration data.

<a id="air-mode-002"></a>
### AIR-MODE-002 — Synthetic-vision mode

Synthetic-vision mode shall be entered only when its declared dependencies are
available and valid; loss of a dependency shall revert deterministically to
conventional horizon mode while preserving higher-priority symbology and alerts.

<a id="air-mode-003"></a>
### AIR-MODE-003 — Unusual-attitude mode

Unusual-attitude mode shall use airframe-profile thresholds and hysteresis,
preserve an unambiguous sky/ground and recovery direction through the full
orientation domain, declutter lower-priority content, and never depend on SVS.

<a id="air-mode-004"></a>
### AIR-MODE-004 — Degraded and reversionary mode

Degraded or reversionary mode shall identify the failed source or function,
retain only valid information, use a predetermined layout and priority, and
avoid automatic transitions whose cause or resulting source is hidden from the
crew.

<a id="air-mode-005"></a>
### AIR-MODE-005 — HUD-SIM mode

HUD-SIM mode shall enable conformal cues only while projection and time-alignment
requirements are valid; loss of calibration shall remove conformal cues or enter
non-conformal repeater mode with an explicit mode annunciation.

<a id="air-mode-006"></a>
### AIR-MODE-006 — Non-conformal repeater mode

Non-conformal repeater mode shall use a layout that cannot be confused with
registered outside-world symbology and shall continuously display its mode and
simulation limitation.

<a id="air-mode-007"></a>
### AIR-MODE-007 — Test and demonstration mode

Harness, injected-data, replay, and demonstration modes shall be visibly
distinguishable from operational data paths and shall preserve **SIM / NOT FOR
FLIGHT** identification in captures and recordings.

## Validity and annunciation flags

<a id="air-flag-001"></a>
### AIR-FLAG-001 — Valid

`Valid` shall mean all required range, format, source, time, quality, integrity,
accuracy, and coherence checks for that signal and intended use have passed.

<a id="air-flag-002"></a>
### AIR-FLAG-002 — Degraded

`Degraded` shall identify a value that remains usable only for its explicitly
declared degraded function, with the limitation visible wherever it matters to
the crew task.

<a id="air-flag-003"></a>
### AIR-FLAG-003 — Stale

`Stale` shall derive from original source or capture time in a declared time
domain and shall never be reset by forwarding, retransmission, or rendering.

<a id="air-flag-004"></a>
### AIR-FLAG-004 — Missing

`Missing` shall identify an input that is not installed or has not been supplied;
the display shall not invent a substitute value with the original label.

<a id="air-flag-005"></a>
### AIR-FLAG-005 — Failed

`Failed` shall identify an input or output that cannot support its intended use
because a validity, integrity, monitor, or availability requirement failed.

<a id="air-flag-006"></a>
### AIR-FLAG-006 — Miscompare

`Miscompare` shall identify disagreement between independent eligible sources
when the declared comparison threshold and persistence are exceeded; source
selection shall not hide the disagreement.

<a id="air-flag-007"></a>
### AIR-FLAG-007 — Simulation and conformality labels

`SIM / NOT FOR FLIGHT`, `HUD-SIM`, and `NON-CONFORMAL / NOT A HUD` labels shall
be rendered by the applicable surface itself, not supplied solely by surrounding
instructions or operator memory.

## Unavailable conditions

<a id="air-unav-001"></a>
### AIR-UNAV-001 — Attitude unavailable

Invalid, non-finite, out-of-range, stale, incoherent, or failed attitude evidence
shall remove normal attitude guidance and present an attitude-failure flag while
leaving independent valid information identifiable.

<a id="air-unav-002"></a>
### AIR-UNAV-002 — Air data or altitude unavailable

Unavailable airspeed, pressure altitude, vertical speed, or barometric reference
shall flag only the affected indication and shall not relabel groundspeed or local
height as the missing air-data quantity.

<a id="air-unav-003"></a>
### AIR-UNAV-003 — Heading or navigation unavailable

Unavailable heading, reference, navigation source, or guidance shall remove or
flag dependent cues while preserving independent track or position data under
its correct label and reference.

<a id="air-unav-004"></a>
### AIR-UNAV-004 — Time or coherence unavailable

Unknown time-domain mapping, source reset, replay, reordering, excessive skew, or
an incomplete atomic snapshot shall make every dependent function unavailable
rather than presenting mutually inconsistent values as one observation.

<a id="air-unav-005"></a>
### AIR-UNAV-005 — Synthetic vision unavailable

Unavailable terrain/obstacle data, database integrity, position, attitude,
coverage, or rendering shall remove SVS content, announce the loss when relevant,
and revert to conventional horizon mode without removing primary alerts.

<a id="air-unav-006"></a>
### AIR-UNAV-006 — Conformal projection unavailable

Unavailable or invalid video time alignment, design eye, intrinsics, extrinsics,
field of view, boresight, or calibration revision shall remove conformal claims
and cues and identify the resulting non-conformal mode.

<a id="air-unav-007"></a>
### AIR-UNAV-007 — Renderer or output unavailable

Command failure, buffer exhaustion, stalled frame generation, backend error,
corrupt output, or failed output-path monitoring shall replace the last-good
image with an explicit display-failure presentation within the allocated time.

<a id="air-unav-008"></a>
### AIR-UNAV-008 — Configuration unavailable

Missing, incompatible, unauthenticated, or corrupt aircraft/display configuration
shall inhibit dependent modes and values and identify the configuration failure;
generic defaults shall not silently acquire aircraft-specific meaning.

## Operating envelope and timing

<a id="air-env-001"></a>
### AIR-ENV-001 — Flight phases

Requirements analysis shall address preflight, taxi, takeoff, climb, cruise,
descent, approach, landing, go-around, postflight, and applicable ground
maintenance; no phase receives operational credit until its crew tasks and
failure effects are validated for the selected aircraft.

<a id="air-env-002"></a>
### AIR-ENV-002 — Attitude envelope

Attitude processing and failure presentation shall remain finite, deterministic,
and unambiguous for pitch and bank through the complete orientation domain,
including inverted flight, vertical attitudes, heading wrap, high angular rate,
and transitions across representation singularities.

<a id="air-env-003"></a>
### AIR-ENV-003 — Aircraft envelope

Speed, altitude, load, angular-rate, acceleration, environmental, and display
viewing envelopes shall come from the selected aircraft and installation; until
selected, browser demonstrations provide no credited envelope and shall flag
values outside their explicitly configured simulation range.

<a id="air-env-004"></a>
### AIR-ENV-004 — Crew tasks and workload

Each mode shall identify pilot flying, pilot monitoring, instructor, maintenance,
or demonstration tasks, expected scan and control transitions, alert response,
and reversion action; human-factors validation shall cover normal, abnormal,
failure, high-workload, and unusual-attitude cases.

<a id="air-tim-001"></a>
### AIR-TIM-001 — Freshness allocation

Freshness limits shall be allocated per signal and function from aircraft-level
latency and hazard analysis, measured from original source or capture time, and
shall define valid, degraded, stale, and failed transitions without extending age
at transport boundaries.

<a id="air-tim-002"></a>
### AIR-TIM-002 — End-to-end latency allocation

The selected installation shall allocate and verify acquisition, transport,
coherence, processing, rendering, scan-out, video, and conformal-overlay latency
and jitter, including worst-case load and failure/recovery behavior.

<a id="air-tim-003"></a>
### AIR-TIM-003 — Reversion and monitor timing

Every unavailable condition shall define a bounded detection, annunciation, and
reversion time derived from its safety assessment; a browser demonstration
threshold shall not be treated as an aircraft allocation.

