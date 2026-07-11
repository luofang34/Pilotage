# Instrument intended functions

## Baseline statement

Pilotage instrument software is an engineering simulator display. Its browser,
WebAssembly, Canvas, network, and Gazebo paths are **SIM / NOT FOR FLIGHT** under
[`AIR-BAS-001`](requirements.md#air-bas-001). The visual vocabulary may support
a later aircraft project, but no output is approved primary flight information,
navigation guidance, terrain alerting, or HUD imagery.

The target architecture uses conservative dual-pilot Part 25 IFR behavior as a
reference under [`AIR-BAS-002`](requirements.md#air-bas-002). The selected
aircraft, certification authority, certification basis, operational rules,
installation, and credited functions remain open. Assurance is therefore
hazard-derived per function rather than assigned to the application as a whole
under [`AIR-BAS-003`](requirements.md#air-bas-003).

## Function matrix

| Function | Intended role in this baseline | Inputs | Output and modes | Failure presentation | Source assumptions |
|---|---|---|---|---|---|
| PFD | Supplemental simulator awareness; architecture reference for a possible PFI function | [`AIR-IN-001`](requirements.md#air-in-001) through [`AIR-IN-009`](requirements.md#air-in-009), alerts under [`AIR-IN-012`](requirements.md#air-in-012), renderer health under [`AIR-IN-013`](requirements.md#air-in-013) | [`AIR-OUT-001`](requirements.md#air-out-001); conventional, unusual-attitude, and degraded/reversionary modes | Per-signal flags and explicit display failure under [`AIR-OUT-004`](requirements.md#air-out-004) | Simulator truth and network telemetry are untrusted until their source, time, range, integrity, and coherence evidence passes the applicable checks |
| HSI | Supplemental simulator navigation awareness; no navigation or approach credit | Heading/navigation, velocity, wind, selections, timing, validity, and renderer health under [`AIR-IN-003`](requirements.md#air-in-003), [`AIR-IN-005`](requirements.md#air-in-005) through [`AIR-IN-009`](requirements.md#air-in-009), and [`AIR-IN-013`](requirements.md#air-in-013) | [`AIR-OUT-002`](requirements.md#air-out-002); conventional and degraded/reversionary modes | Heading/navigation loss under [`AIR-UNAV-003`](requirements.md#air-unav-003), time/coherence loss, and display failure | No magnetic/true reference, datum, navigation mode, or integrity is inferred from a local yaw or unlabeled simulator field |
| Conventional instruments | Simulator comparison/reversion surface; no independent standby credit | The validated sources used by the corresponding PFD quantities | [`AIR-OUT-003`](requirements.md#air-out-003); conventional and degraded/reversionary modes | Each instrument flags its own missing, stale, failed, or miscompared source | Drawing a separate instrument does not establish source, power, processing, or display independence |
| SVS | Planned removable background supplying supplemental situation awareness only | Validated position, attitude, timing, databases, selections, and renderer health under [`AIR-IN-001`](requirements.md#air-in-001), [`AIR-IN-002`](requirements.md#air-in-002), [`AIR-IN-008`](requirements.md#air-in-008), [`AIR-IN-009`](requirements.md#air-in-009), [`AIR-IN-011`](requirements.md#air-in-011), and [`AIR-IN-013`](requirements.md#air-in-013) | [`AIR-OUT-005`](requirements.md#air-out-005); synthetic-vision mode with deterministic conventional-horizon reversion | [`AIR-UNAV-005`](requirements.md#air-unav-005); primary symbology and external alerts remain available if their own paths are valid | Database graphics are not sensor observations, TAWS, or assurance evidence; each database and navigation dependency must be independently valid |
| SVGS | Not supplied | None accepted by this baseline | No guidance output and no low-visibility operational credit under [`AIR-OUT-009`](requirements.md#air-out-009) | No silent fallback to an SVGS-like pathway | A future function requires a separate intended function and applicable accepted performance standard |
| HUD-SIM | Planned fixed-design-eye conformal simulator overlay; no airborne HUD credit | Validated flight state plus video/calibration under [`AIR-IN-010`](requirements.md#air-in-010) | [`AIR-OUT-006`](requirements.md#air-out-006) in HUD-SIM mode | Remove conformal cues or revert to a conspicuously non-conformal mode under [`AIR-UNAV-006`](requirements.md#air-unav-006) | Conformality exists only for the declared camera, design eye, calibration revision, FOV, pose, and time alignment |
| Non-conformal repeater | Planned simulator display when conformal prerequisites are absent | Validated flight state; no camera calibration is claimed | [`AIR-OUT-007`](requirements.md#air-out-007) in non-conformal repeater mode | Persistent **NON-CONFORMAL / NOT A HUD** identification; invalid flight signals retain their own flags | Similar color or stroke symbology does not make a repeater an optical or conformal HUD |
| Airborne optical HUD/HWD | Outside boundary | Not specified | No output or operational credit under [`AIR-OUT-008`](requirements.md#air-out-008) | Not applicable | Optical, installation, alignment, continued-airworthiness, and human-factors evidence belong to an aircraft project |
| TAWS alerts | External future input, not an SVS function | Independently monitored alert input under [`AIR-IN-012`](requirements.md#air-in-012) | Display/aural annunciation only after a separate alert-management intended function is defined | Alert loss or invalidity must not be hidden by terrain graphics | SVS shall remain independent of TAWS under [`AIR-OUT-010`](requirements.md#air-out-010) |

## Modes and deterministic reversion

The defined display modes are conventional horizon, synthetic vision,
unusual attitude, degraded/reversionary, HUD-SIM, non-conformal repeater, and
test/demonstration. Their requirements are
[`AIR-MODE-001`](requirements.md#air-mode-001) through
[`AIR-MODE-007`](requirements.md#air-mode-007).

Mode changes are explicit display state, not incidental renderer behavior. The
same coherent snapshot and configuration produces the same transition. An SVS
failure returns to the conventional sky/ground background. A conformal-
calibration failure removes conformal cues or enters non-conformal repeater
mode. An input failure flags the affected data. A renderer/output failure
replaces the last-good frame with a display-failure presentation. Reversion
never upgrades an unverified source or hides the reason for the transition.

| Operating condition | Entered state | Required crew-visible result |
|---|---|---|
| Normal, validated basic inputs | Conventional horizon, HSI, or conventional instrument mode | Declared source/reference and **SIM / NOT FOR FLIGHT** label remain visible |
| Normal, all SVS dependencies valid | Synthetic-vision mode if selected | SVS remains below primary symbology; loss returns to conventional horizon |
| Normal, all conformal dependencies valid | HUD-SIM mode if selected | Conformal cues and **HUD-SIM / NOT FOR FLIGHT** mode are visible |
| Abnormal aircraft attitude | Unusual-attitude mode | Unambiguous sky/ground, recovery direction, and priority declutter remain independent of SVS |
| Degraded, stale, missing, or miscompared source | Degraded/reversionary mode | Affected data is flagged; remaining data keeps its identity and reference |
| Invalid conformal dependency | Non-conformal repeater or cue removal | **NON-CONFORMAL / NOT A HUD** is visible and no conformal claim remains |
| Renderer, command, buffer, output, or progress-monitor failure | Display failure | The retained last-good image is replaced within the allocated monitor time |

## Validity and unavailable behavior

The only baseline validity classes are `Valid`, `Degraded`, `Stale`, `Missing`,
`Failed`, and `Miscompare`, defined by
[`AIR-FLAG-001`](requirements.md#air-flag-001) through
[`AIR-FLAG-006`](requirements.md#air-flag-006). Unknown values, absent metadata,
non-finite values, impossible ranges, incoherent snapshots, and unsupported
enumerations fail closed. Labels required by
[`AIR-FLAG-007`](requirements.md#air-flag-007) remain part of the rendered
surface.

Each function uses the unavailable conditions in
[`AIR-UNAV-001`](requirements.md#air-unav-001) through
[`AIR-UNAV-008`](requirements.md#air-unav-008). Independent valid information
may remain, but the display never represents local height as pressure altitude,
groundspeed as indicated airspeed, track as heading, retransmission time as
source time, SVS as TAWS, or a repeater as HUD.

## Operational scope

Flight phases and crew tasks requiring analysis are defined by
[`AIR-ENV-001`](requirements.md#air-env-001) and
[`AIR-ENV-004`](requirements.md#air-env-004). The complete orientation domain is
inside the robustness and failure-presentation scope under
[`AIR-ENV-002`](requirements.md#air-env-002); this includes vertical and inverted
attitudes even when an aircraft later prohibits intentional operation there.

No credited speed, altitude, load, angular-rate, acceleration, environmental,
or viewing envelope exists until selected under
[`AIR-ENV-003`](requirements.md#air-env-003). Likewise, freshness, end-to-end
latency, display-monitor, and reversion timing are allocations to be derived,
not inherited from simulator constants; see
[`AIR-TIM-001`](requirements.md#air-tim-001) through
[`AIR-TIM-003`](requirements.md#air-tim-003).

## Flight phases and crew tasks

The following table scopes analysis and simulation test cases; it grants no
operational credit. “PF” and “PM” are reference roles for a possible dual-pilot
aircraft project. Simulator operation uses an operator and may use an instructor.

| Phase | Reference display task | Exclusion or required follow-on evidence |
|---|---|---|
| Preflight / postflight | Operator verifies simulation mode, sources, flags, configuration, databases, and test setup | No dispatch, built-in-test, or maintenance-release credit |
| Taxi | Operator maintains ground orientation and monitors declared navigation/SVS data | No SafeTaxi, runway-incursion, low-visibility taxi, or external-visual-reference credit |
| Takeoff / go-around | PF reference task is attitude, airspeed, altitude, vertical trend, mode, and alert scan; PM reference task is source/mode cross-check | No flight-director, command guidance, V-speed, takeoff-performance, or HUD credit without separate intended functions and validation |
| Climb / cruise / descent | PF reference task is flight-path control scan; PM reference task is navigation/source, trend, mode, and alert monitoring | Air-data, heading, navigation, and alert sources remain uncredited simulator inputs |
| Approach / landing | PF/PM reference tasks include approach-source, deviation, altitude, mode, and alert cross-check | No approach, landing, EFVS, SVGS, minima, or low-visibility credit |
| Unusual attitude / high workload | PF reference task is recognize attitude and recover; PM reference task is call out mode/source failures and monitor recovery | Thresholds, declutter, recovery cues, recognition time, workload, and transition behavior require aircraft-specific human-factors validation |
| Source or display failure | PF reference task is maintain control from valid independent information; PM reference task is identify the failed source/mode and execute the selected reversion procedure | No independence, redundancy, dispatch, or reversion credit until the aircraft architecture and failure assessment establish it |
| Maintenance / test / replay | Maintainer or instructor identifies injected data, test mode, configuration, and expected failure response | Test inputs cannot enter an operational path without explicit inhibition and indication |
