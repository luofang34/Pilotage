# Preliminary functional hazard assessment (AIR-02)

**SIM / NOT FOR FLIGHT.** This is a preliminary functional hazard assessment
(FHA) of the Pilotage instrument display functions. It is an engineering input
to a future safety assessment, not an FAA/EASA finding, TSO authorization, or
TC/STC approval. Every browser, WebAssembly, Canvas, WebTransport, Gazebo, and
test-harness surface analysed here is **SIM / NOT FOR FLIGHT** under
[`AIR-BAS-001`](requirements.md#air-bas-001).

This assessment is **preliminary** and is not closable. Its classifications
remain conditional on decisions this project has not made — the target vehicle,
operation, installation, credited function, and certification basis — per
[`AIR-HAZ-001`](requirements.md#air-haz-001). Closure requires the qualified
independent safety review recorded in the [PSSA](pssa.md); the review fields
there remain `PENDING`.

## Scope and inputs

The analysed functions and their boundary are the merged AIR-01 baseline:
[intended functions](intended-functions.md), [system boundary](system-boundary.md),
and the [requirements registry](requirements.md). The architecture context is
[ADR-0017](../adr/0017-instrument-display-runtime.md) (no_std sans-IO runtime,
scene-command IR, six criticality-banded compositor layers, first-class
validity) and [ADR-0018](../adr/0018-avionics-telemetry-and-aviate-adapter.md)
(measurement-identity ingress gate, coherence result, fail-closed authorization).

The hazard set additionally covers the vehicle-neutral six-degree-of-freedom
frame contract introduced by FRAME-01 (issue #52), whose frame, epoch, clock,
and time-scale identities create hazards an aircraft-only display does not have.

The [PSSA](pssa.md) carries the common-cause and independence analysis, the
assurance allocation, the simulator-versus-airborne mitigation split, and the
full bidirectional traceability from these failure conditions to implementation
and verification issues.

## Method and conventions

Failure conditions are analysed at the **display level** — the effect on what
the crew sees and does — because the sensors, estimators, alerting logic, and
installation that would set aircraft-level effects are outside this boundary
(see [system boundary](system-boundary.md)). Aircraft-level classification is a
downstream activity performed against a selected certification basis.

Each failure condition records the flight phase where it is most critical (the
phases are those of [`AIR-ENV-001`](requirements.md#air-env-001)), the
display-level and crew effect, the detection means and any independence
assumption, a preliminary conditional severity, and the derived safety
requirement(s).

### Failure-condition classes analysed

Per issue #27 and the intended-function baseline, every function is analysed for
**loss** (missing), **misleading** (hazardously incorrect but plausible),
**frozen** (retained last-good), **stale/latent** (aged or lagging), **reordered
or replayed** data, **wrong reference** (datum, polarity, heading, or frame),
and **failed reversion**, plus the cross-cutting **compositor**, **renderer or
output**, **monitor**, **timebase**, **source-selection**, and **common-cause**
failure conditions.

### Conditional severity notation

Severity vocabulary is the qualitative ARP4761A / AC 25.1309-1B set —
Catastrophic, Hazardous, Major, Minor, No Safety Effect. Unconditional severities
are impossible before a vehicle and operation are selected
([`AIR-HAZ-001`](requirements.md#air-haz-001)), so each entry gives two explicitly
conditional anchors:

- **WCC (worst-credible-if-credited):** the plausible worst severity *if* this
  function were credited primary flight information on the conservative
  dual-pilot Part 25 IFR reference of
  [`AIR-BAS-002`](requirements.md#air-bas-002), in instrument meteorological
  conditions, at the most exposed phase. It ranks analysis and assurance
  priority only.
- **ABT (as-built-today):** the effect in the only configuration that exists —
  supplemental **SIM / NOT FOR FLIGHT** awareness with no operational credit —
  which is **No Safety Effect** on any real operation.

Both anchors are conditional. A spacecraft, rotorcraft, Part 23, or
uncrewed-operation basis would move them, and only a selected basis with an
aircraft-level assessment produces a real classification. Where an FRAME-01
spacecraft operation changes the analysis, it is stated in the entry.

## Failure-condition hazard log

### Attitude and primary flight display

| ID | Failure condition | Display-level & crew effect | Detection / independence assumption | Conditional severity (WCC / ABT) | Derived requirement(s) |
|---|---|---|---|---|---|
| FC-ATT-01 | Loss of attitude | **All phases.** Attitude guidance removed; attitude-failure flag shown; independent data retained | Per-signal validity and range checks; fails to a flag, not a value | Hazardous–Catastrophic / No Safety Effect | [`AIR-UNAV-001`](requirements.md#air-unav-001), [`AIR-HAZ-002`](requirements.md#air-haz-002) |
| FC-ATT-02 | Misleading attitude (plausible wrong pitch/bank accepted) | **Takeoff, approach, unusual-attitude.** Crew flies to a false horizon; control input in wrong sense; unrecognised | Requires cross-source comparison; single-source attitude has no on-display detection | Catastrophic / No Safety Effect | [`AIR-IN-001`](requirements.md#air-in-001), [`AIR-FLAG-006`](requirements.md#air-flag-006), [`AIR-HAZ-002`](requirements.md#air-haz-002) |
| FC-ATT-03 | Frozen horizon (last-good retained through maneuver) | **All phases.** False steady attitude while vehicle rotates; delayed recognition | Independent renderer-progress and source-time evidence; frozen frame must be replaced | Catastrophic / No Safety Effect | [`AIR-UNAV-007`](requirements.md#air-unav-007), [`AIR-IN-013`](requirements.md#air-in-013), [`AIR-HAZ-011`](requirements.md#air-haz-011) |
| FC-ATT-04 | Stale / latent attitude (aged, not flagged) | **Unusual-attitude, high-rate.** Lagged horizon; overcontrol or late recovery | Freshness derived from source time; staleness thresholds allocated per function | Hazardous / No Safety Effect | [`AIR-FLAG-003`](requirements.md#air-flag-003), [`AIR-TIM-001`](requirements.md#air-tim-001), [`AIR-HAZ-011`](requirements.md#air-haz-011) |
| FC-ATT-05 | Wrong reference frame / absent frame metadata | **All phases.** Horizon referenced to the wrong body/navigation frame or an assumed local vertical; tilt or datum wrong | Explicit declared frames; missing metadata must drive unavailable | Catastrophic / No Safety Effect | [`AIR-BAS-004`](requirements.md#air-bas-004), [`AIR-IN-001`](requirements.md#air-in-001), [`AIR-HAZ-006`](requirements.md#air-haz-006), [`AIR-HAZ-007`](requirements.md#air-haz-007) |
| FC-ATT-06 | Polarity / singularity ambiguity | **Unusual-attitude, inverted, vertical, heading wrap.** Wrong recovery direction or ambiguous sky/ground at representation singularities | Full-orientation-domain determinism; SO(3)-safe presentation | Catastrophic / No Safety Effect | [`AIR-ENV-002`](requirements.md#air-env-002), [`AIR-MODE-003`](requirements.md#air-mode-003), [`AIR-HAZ-012`](requirements.md#air-haz-012) |
| FC-ATT-07 | Failed reversion / declutter (SVS bleeds into unusual-attitude) | **Unusual-attitude, high-workload.** Cluttered or ambiguous recovery picture; background not shed | Deterministic reversion; unusual-attitude independent of SVS | Catastrophic / No Safety Effect | [`AIR-MODE-003`](requirements.md#air-mode-003), [`AIR-MODE-004`](requirements.md#air-mode-004), [`AIR-HAZ-005`](requirements.md#air-haz-005) |

### Air data and altitude

| ID | Failure condition | Display-level & crew effect | Detection / independence assumption | Conditional severity (WCC / ABT) | Derived requirement(s) |
|---|---|---|---|---|---|
| FC-AD-01 | Loss of airspeed / altitude | **All phases.** Affected tape flagged; honest `Missing`; no fabricated value | Presence is structural; a never-supplied signal reads `Missing` | Major–Hazardous / No Safety Effect | [`AIR-UNAV-002`](requirements.md#air-unav-002), [`AIR-FLAG-004`](requirements.md#air-flag-004) |
| FC-AD-02 | Misleading airspeed / altitude value | **Takeoff, approach, landing.** Unrecognised over/underspeed or altitude error | Requires comparison or integrity; single-source air data has no on-display detection | Catastrophic / No Safety Effect | [`AIR-IN-004`](requirements.md#air-in-004), [`AIR-FLAG-006`](requirements.md#air-flag-006), [`AIR-HAZ-002`](requirements.md#air-haz-002) |
| FC-AD-03 | Wrong datum / barometric setting (or NED height labelled pressure altitude) | **Approach, landing.** Altitude reads against wrong datum; terrain-clearance error | Declared altitude type/datum; missing datum drives unavailable | Catastrophic / No Safety Effect | [`AIR-IN-002`](requirements.md#air-in-002), [`AIR-UNAV-002`](requirements.md#air-unav-002), [`AIR-HAZ-007`](requirements.md#air-haz-007) |
| FC-AD-04 | Frozen airspeed / altitude tape | **All phases.** False steady value while state changes | Renderer-progress and source-time evidence | Hazardous–Catastrophic / No Safety Effect | [`AIR-UNAV-007`](requirements.md#air-unav-007), [`AIR-HAZ-011`](requirements.md#air-haz-011) |
| FC-AD-05 | Vertical-speed polarity inverted | **Climb, descent, approach.** Climb shown as descent or vice versa | Sign integrity through derivation and reversion | Hazardous / No Safety Effect | [`AIR-HAZ-012`](requirements.md#air-haz-012), [`AIR-IN-004`](requirements.md#air-in-004) |
| FC-AD-06 | Substitution (groundspeed relabelled as indicated airspeed on air-data loss) | **Takeoff, approach.** Ground-relative speed read as air-relative; stall/overspeed margin wrong | Never relabel across quantities; loss flags the affected indication only | Catastrophic / No Safety Effect | [`AIR-UNAV-002`](requirements.md#air-unav-002), [`AIR-HAZ-002`](requirements.md#air-haz-002) |

### Heading, navigation, and horizontal situation

| ID | Failure condition | Display-level & crew effect | Detection / independence assumption | Conditional severity (WCC / ABT) | Derived requirement(s) |
|---|---|---|---|---|---|
| FC-HDG-01 | Loss of heading / navigation | **Taxi, approach.** Dependent cues removed or flagged; independent track/position kept | Per-signal validity; independent data retains its own label | Major–Hazardous / No Safety Effect | [`AIR-UNAV-003`](requirements.md#air-unav-003), [`AIR-OUT-002`](requirements.md#air-out-002) |
| FC-HDG-02 | Wrong heading reference (magnetic/true confusion, or body yaw shown as heading) | **All phases.** Heading off by variation or by sideslip; wrong track flown | Declared true/magnetic reference and variation source; heading separate from attitude yaw | Hazardous / No Safety Effect | [`AIR-IN-005`](requirements.md#air-in-005), [`AIR-BAS-004`](requirements.md#air-bas-004), [`AIR-HAZ-007`](requirements.md#air-haz-007) |
| FC-HDG-03 | Misleading deviation (wrong polarity or scale) | **Approach.** Crew flies to the wrong side of course/glidepath | Sign and scale integrity; declared navigation mode | Catastrophic / No Safety Effect | [`AIR-IN-005`](requirements.md#air-in-005), [`AIR-HAZ-012`](requirements.md#air-haz-012) |
| FC-HDG-04 | Frozen / stale navigation | **Approach, taxi.** False position, track, or distance | Source-time freshness and renderer progress | Hazardous / No Safety Effect | [`AIR-HAZ-011`](requirements.md#air-haz-011), [`AIR-FLAG-003`](requirements.md#air-flag-003) |
| FC-HDG-05 | Undeclared navigation mode / source | **Approach.** Crew credits guidance from the wrong or an invalid source | Declared navigation source and mode; reversion identifies the source | Hazardous / No Safety Effect | [`AIR-IN-005`](requirements.md#air-in-005), [`AIR-MODE-004`](requirements.md#air-mode-004) |

### Conventional instruments (comparison / reversion surface)

| ID | Failure condition | Display-level & crew effect | Detection / independence assumption | Conditional severity (WCC / ABT) | Derived requirement(s) |
|---|---|---|---|---|---|
| FC-INST-01 | False standby independence (six-pack drawn from same state treated as independent) | **Source or display failure.** A common-cause fault takes the "standby" down with the primary; unwarranted redundancy credit | Independence is not established by drawing a second instrument; requires common-cause analysis | Catastrophic / No Safety Effect | [`AIR-OUT-003`](requirements.md#air-out-003), [`AIR-HAZ-009`](requirements.md#air-haz-009) |
| FC-INST-02 | Conventional instrument fabricates a missing source | **All phases.** A dial shows a plausible value with no valid source | Missing/failed source flags the instrument; no last-good substitution | Catastrophic / No Safety Effect | [`AIR-OUT-003`](requirements.md#air-out-003), [`AIR-OUT-004`](requirements.md#air-out-004), [`AIR-HAZ-002`](requirements.md#air-haz-002) |

### Synthetic vision

| ID | Failure condition | Display-level & crew effect | Detection / independence assumption | Conditional severity (WCC / ABT) | Derived requirement(s) |
|---|---|---|---|---|---|
| FC-SVS-01 | False terrain / runway (database corruption or coverage gap) | **Approach, taxi, low-level.** Wrong terrain or runway painted; false clearance impression | Database integrity, coverage, and provenance evidence; SVS is supplemental, not TAWS | Hazardous–Catastrophic / No Safety Effect | [`AIR-IN-011`](requirements.md#air-in-011), [`AIR-OUT-005`](requirements.md#air-out-005), [`AIR-UNAV-005`](requirements.md#air-unav-005) |
| FC-SVS-02 | Lost position integrity (scene registered to wrong position) | **Approach.** Terrain/runway misregistered under symbology; misleading conformal impression | Position integrity and coherence with attitude/time; fails to remove SVS | Hazardous / No Safety Effect | [`AIR-IN-011`](requirements.md#air-in-011), [`AIR-UNAV-005`](requirements.md#air-unav-005), [`AIR-HAZ-006`](requirements.md#air-haz-006) |
| FC-SVS-03 | SVS obscures primary symbology (compositor priority error) | **All phases.** Background covers attitude/tapes/alerts | Compositor enforces `background < symbology < warning`; fails toward exposing critical bands | Catastrophic / No Safety Effect | [`AIR-OUT-005`](requirements.md#air-out-005), [`AIR-HAZ-004`](requirements.md#air-haz-004) |
| FC-SVS-04 | SVS mistaken for or suppressing terrain alerting | **Approach, low-level.** Crew reads terrain shading as a terrain-clear/alert function | SVS and TAWS remain visually and functionally distinct; alert loss is not hidden by graphics | Catastrophic / No Safety Effect | [`AIR-OUT-010`](requirements.md#air-out-010), [`AIR-IN-012`](requirements.md#air-in-012) |
| FC-SVS-05 | Failed reversion (SVS fault does not return to conventional horizon) | **All phases.** Frozen or black background instead of sky/ground | Deterministic reversion to horizon; primary alerts preserved | Hazardous–Catastrophic / No Safety Effect | [`AIR-UNAV-005`](requirements.md#air-unav-005), [`AIR-MODE-002`](requirements.md#air-mode-002), [`AIR-HAZ-005`](requirements.md#air-haz-005) |
| FC-SVS-06 | Frozen last-good imagery (raster renderer retains last terrain frame) | **All phases.** Stale terrain shown as current | Independent raster-renderer progress evidence; fault-contained SVS renderer | Hazardous / No Safety Effect | [`AIR-HAZ-011`](requirements.md#air-haz-011), [`AIR-UNAV-007`](requirements.md#air-unav-007) |

### HUD-SIM and non-conformal repeater

| ID | Failure condition | Display-level & crew effect | Detection / independence assumption | Conditional severity (WCC / ABT) | Derived requirement(s) |
|---|---|---|---|---|---|
| FC-HUD-01 | Conformal misregistration (calibration / extrinsics error) | **Approach, takeoff.** Conformal cue points to the wrong world position; misleading guidance impression | Calibration revision, design eye, intrinsics/extrinsics validated; invalid removes conformal claim | Hazardous–Catastrophic / No Safety Effect | [`AIR-IN-010`](requirements.md#air-in-010), [`AIR-OUT-006`](requirements.md#air-out-006), [`AIR-UNAV-006`](requirements.md#air-unav-006) |
| FC-HUD-02 | Video / state time skew (overlay lags imagery) | **Approach.** Symbology misregisters against video under motion | Capture-time and state-time alignment within a latency budget | Hazardous / No Safety Effect | [`AIR-IN-010`](requirements.md#air-in-010), [`AIR-TIM-002`](requirements.md#air-tim-002), [`AIR-HAZ-011`](requirements.md#air-haz-011) |
| FC-HUD-03 | Repeater mistaken for conformal HUD (missing NON-CONFORMAL identity) | **All phases.** Non-conformal repeater read as registered outside-world guidance | Persistent **NON-CONFORMAL / NOT A HUD** identity rendered by the surface itself | Hazardous / No Safety Effect | [`AIR-OUT-007`](requirements.md#air-out-007), [`AIR-MODE-006`](requirements.md#air-mode-006), [`AIR-FLAG-007`](requirements.md#air-flag-007) |
| FC-HUD-04 | Failed conformal reversion (calibration invalid, conformal cues retained) | **Approach.** Stale or invalid conformal cues persist after calibration loss | Loss of calibration removes cues or enters non-conformal mode with annunciation | Hazardous–Catastrophic / No Safety Effect | [`AIR-UNAV-006`](requirements.md#air-unav-006), [`AIR-MODE-005`](requirements.md#air-mode-005), [`AIR-HAZ-005`](requirements.md#air-haz-005) |

### Vehicle frame and six-degree-of-freedom state (FRAME-01)

These failure conditions come from the vehicle-neutral frame contract of issue
#52. Their aircraft severities use the NED/local-vertical adapter; the spacecraft
column is stated where the operation changes the effect. All remain conditional
on a selected vehicle and operation
([`AIR-HAZ-001`](requirements.md#air-haz-001)).

| ID | Failure condition | Display-level & crew effect | Detection / independence assumption | Conditional severity (WCC / ABT) | Derived requirement(s) |
|---|---|---|---|---|---|
| FC-FRM-01 | Wrong inertial / local frame selected | **All phases.** Attitude or velocity presented in the wrong frame; horizon or vector points wrong | Typed frame identity on every quantity; composition is a checked operation | Catastrophic / No Safety Effect | [`AIR-HAZ-006`](requirements.md#air-haz-006), [`AIR-HAZ-007`](requirements.md#air-haz-007) |
| FC-FRM-02 | Stale transform epoch (outdated frame relationship composed) | **Maneuver, orbital ops.** Transform uses an expired frame relationship; misregistered presentation | Epoch carried and checked; mismatched epoch fails closed | Hazardous–Catastrophic / No Safety Effect | [`AIR-HAZ-006`](requirements.md#air-haz-006), [`AIR-UNAV-004`](requirements.md#air-unav-004) |
| FC-FRM-03 | Clock / time-scale mismatch across groups | **All phases.** Values from different time scales blended and shown as one observation | Declared clock domain and time-scale; incoherent snapshot drives unavailable | Catastrophic / No Safety Effect | [`AIR-UNAV-004`](requirements.md#air-unav-004), [`AIR-IN-008`](requirements.md#air-in-008), [`AIR-HAZ-006`](requirements.md#air-haz-006) |
| FC-FRM-04 | LVLH / inertial confusion | **Orbital ops.** Orbital attitude read in the wrong basis; wrong pointing understanding | Distinct, labelled, visually distinguishable projections; basis change is annunciated | Hazardous–Catastrophic (op-dependent) / No Safety Effect | [`AIR-HAZ-008`](requirements.md#air-haz-008), [`AIR-HAZ-007`](requirements.md#air-haz-007) |
| FC-FRM-05 | Invalid local-vertical assumption (horizon fabricated where none defined) | **Orbital / zero-g, high-altitude.** A horizon is invented where the frame defines no local vertical | Attitude remains meaningful with no gravity/horizon; no fabricated horizon | Hazardous / No Safety Effect | [`AIR-HAZ-007`](requirements.md#air-haz-007), [`AIR-ENV-002`](requirements.md#air-env-002) |

### Timebase, source selection, compositor, renderer, and monitor

| ID | Failure condition | Display-level & crew effect | Detection / independence assumption | Conditional severity (WCC / ABT) | Derived requirement(s) |
|---|---|---|---|---|---|
| FC-TIME-01 | Clock reset / skew undetected | **All phases.** Stale data shown as fresh; replay accepted | Source epoch, monotonic sequence, and time-domain mapping detect reset/replay/age | Catastrophic / No Safety Effect | [`AIR-IN-008`](requirements.md#air-in-008), [`AIR-UNAV-004`](requirements.md#air-unav-004), [`AIR-HAZ-011`](requirements.md#air-haz-011) |
| FC-TIME-02 | Reordering / replay accepted into display state | **All phases.** Out-of-order or replayed values presented | Wrap-safe sequence and epoch ordering reject duplicates and reorderings | Hazardous / No Safety Effect | [`AIR-IN-008`](requirements.md#air-in-008), [`AIR-UNAV-004`](requirements.md#air-unav-004) |
| FC-SRC-01 | Miscompare hidden by source selection | **All phases.** One lying source presented as valid; disagreement suppressed | Miscompare exposed when threshold/persistence exceeded; selection cannot hide it | Catastrophic / No Safety Effect | [`AIR-FLAG-006`](requirements.md#air-flag-006), [`AIR-HAZ-002`](requirements.md#air-haz-002) |
| FC-SRC-02 | Cross-channel common-cause defeats comparison | **All phases.** Both channels fail the same way; comparison passes while both are wrong | Requires documented common-cause boundary; monitors must not share the fault | Catastrophic / No Safety Effect | [`AIR-HAZ-009`](requirements.md#air-haz-009), [`AIR-HAZ-003`](requirements.md#air-haz-003) |
| FC-CMP-01 | Compositor priority-table error | **All phases.** Background painted above primary/warning bands | Encoding order is z-order; unknown layer id fails the frame; fault favors exposing critical | Catastrophic / No Safety Effect | [`AIR-HAZ-004`](requirements.md#air-haz-004), [`AIR-BAS-007`](requirements.md#air-bas-007) |
| FC-CMP-02 | Compositor resource exhaustion drops a critical layer | **All phases.** Primary symbology or warnings missing from the composed frame | Per-layer/frame budgets; missing critical band fails toward display-failure | Catastrophic / No Safety Effect | [`AIR-HAZ-004`](requirements.md#air-haz-004), [`AIR-UNAV-007`](requirements.md#air-unav-007) |
| FC-CMP-03 | Layer misattribution (content in wrong criticality band) | **All phases.** Content painted at the wrong z-order or criticality | Every command inside exactly one layer; validated ascending, unnested layers | Hazardous–Catastrophic / No Safety Effect | [`AIR-HAZ-004`](requirements.md#air-haz-004), [`AIR-BAS-007`](requirements.md#air-bas-007) |
| FC-RND-01 | Renderer stall retains last-good image | **All phases.** Whole display frozen; no indication | Independent progress/output monitor replaces the retained frame within allocated time | Catastrophic / No Safety Effect | [`AIR-UNAV-007`](requirements.md#air-unav-007), [`AIR-IN-013`](requirements.md#air-in-013), [`AIR-HAZ-011`](requirements.md#air-haz-011) |
| FC-RND-02 | Corrupt command / output buffer | **All phases.** Garbled or partial symbology | Buffer integrity and backend status; corrupt output drives display-failure | Hazardous–Catastrophic / No Safety Effect | [`AIR-UNAV-007`](requirements.md#air-unav-007), [`AIR-OUT-004`](requirements.md#air-out-004) |
| FC-RND-03 | Nondeterministic backend (same state renders differently) | **All phases.** Layout/glyph/raster drift between platforms; unrepeatable presentation | Reproducible glyph pack and reference rasterizer; deterministic presentation | Major–Hazardous / No Safety Effect | [`AIR-BAS-007`](requirements.md#air-bas-007), [`AIR-FLAG-007`](requirements.md#air-flag-007) |
| FC-MON-01 | Monitor coverage gap (stall not detected) | **All phases.** Frozen/failed display not caught | Monitor coverage sized to the failure conditions it must detect; bounded exposure | Catastrophic / No Safety Effect | [`AIR-IN-013`](requirements.md#air-in-013), [`AIR-HAZ-003`](requirements.md#air-haz-003), [`AIR-HAZ-010`](requirements.md#air-haz-010) |
| FC-MON-02 | Monitor shares fault with the path it monitors | **All phases.** Monitor and renderer fail together; no detection | Monitor independence from monitored computation, timebase, power | Catastrophic / No Safety Effect | [`AIR-HAZ-003`](requirements.md#air-haz-003), [`AIR-HAZ-009`](requirements.md#air-haz-009) |
| FC-MON-03 | Latent monitor failure (undetected until demanded) | **Source or display failure.** No coverage when a fault finally occurs | Power-up test, continuous monitor, or periodic self-test bounding exposure | Catastrophic / No Safety Effect | [`AIR-HAZ-010`](requirements.md#air-haz-010), [`AIR-TIM-003`](requirements.md#air-tim-003) |

### Common-cause and combination failure conditions

These combinations are the reason assurance is allocated by architecture, not by
crate. The [PSSA](pssa.md) develops the common-cause boundaries.

| ID | Failure condition | Display-level & crew effect | Detection / independence assumption | Conditional severity (WCC / ABT) | Derived requirement(s) |
|---|---|---|---|---|---|
| FC-CC-01 | Shared timebase fault | **All phases.** All time-dependent functions stale together; coherence and comparison both defeated | Timebase is a declared common-cause boundary; coherence uses independent evidence | Catastrophic / No Safety Effect | [`AIR-HAZ-009`](requirements.md#air-haz-009), [`AIR-UNAV-004`](requirements.md#air-unav-004) |
| FC-CC-02 | Shared validated state copy | **Source or display failure.** "Independent" instruments and standby all wrong together | The same state copy establishes no independence | Catastrophic / No Safety Effect | [`AIR-HAZ-009`](requirements.md#air-haz-009), [`AIR-OUT-003`](requirements.md#air-out-003) |
| FC-CC-03 | Shared power / renderer / compositor | **All phases.** Total display loss including the reversion surface | Independence of the reversion path from the primary path | Catastrophic / No Safety Effect | [`AIR-HAZ-009`](requirements.md#air-haz-009), [`AIR-HAZ-003`](requirements.md#air-haz-003) |
| FC-CC-04 | Shared database / position source | **Approach, low-level.** SVS and every position-derived cue wrong together | Database and position provenance treated as a common-cause boundary | Hazardous–Catastrophic / No Safety Effect | [`AIR-HAZ-009`](requirements.md#air-haz-009), [`AIR-IN-011`](requirements.md#air-in-011) |

## Hazard count by category

| Category | Failure conditions |
|---|---|
| Attitude / PFD | 7 |
| Air data / altitude | 6 |
| Heading / navigation / HSI | 5 |
| Conventional instruments | 2 |
| Synthetic vision | 6 |
| HUD-SIM / repeater | 4 |
| Vehicle frame / six-DoF (FRAME-01) | 5 |
| Timebase | 2 |
| Source selection | 2 |
| Compositor | 3 |
| Renderer / output | 3 |
| Monitor | 3 |
| Common-cause combinations | 4 |
| **Total** | **52** |

## Derived safety requirements

The hazard log derives safety requirements of two kinds:

- **Existing intended-function requirements** (`AIR-BAS`, `AIR-IN`, `AIR-OUT`,
  `AIR-MODE`, `AIR-FLAG`, `AIR-UNAV`, `AIR-ENV`, `AIR-TIM`) that already discharge
  a failure condition; the FHA confirms their safety role and the [PSSA](pssa.md)
  allocates them to implementation and verification issues.
- **New hazard-derived requirements** minted by this assessment,
  [`AIR-HAZ-001`](requirements.md#air-haz-001) through
  [`AIR-HAZ-012`](requirements.md#air-haz-012), which close gaps the
  intended-function baseline did not state as normative requirements —
  misleading-information detection, monitor independence, compositor fail-safe
  priority, fail-safe self-annunciating reversion, frame/epoch/time-scale
  coherence, explicit reference basis, reference-frame confusion barrier,
  common-cause declaration, latent-failure exposure, frozen/stale detection at
  source time, and polarity/ordering integrity.

The bidirectional traceability from every failure condition and derived
requirement to an implementation or verification issue — and the explicit
**unallocated** table for derived requirements with no existing issue — is in the
[PSSA](pssa.md). This FHA is not closable; classification and closure depend on a
selected certification basis and the qualified independent safety review recorded
there.
