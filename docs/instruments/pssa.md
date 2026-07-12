# Preliminary system safety assessment (AIR-02)

**SIM / NOT FOR FLIGHT.** This is a preliminary system safety assessment (PSSA)
of the Pilotage instrument display architecture. It develops the common-cause and
independence analysis, the assurance allocation, the simulator-versus-airborne
mitigation split, and the bidirectional traceability for the failure conditions
identified in the [FHA](fha.md). It is an engineering input to a future safety
assessment, not an FAA/EASA finding, TSO authorization, or TC/STC approval. Every
surface analysed remains **SIM / NOT FOR FLIGHT** under
[`AIR-BAS-001`](requirements.md#air-bas-001).

This assessment is **preliminary and not closable**. No development assurance
level, integrity level, or quantitative probability objective is assigned, and
none may be, until the target vehicle, operation, installation, credited function,
and certification basis are selected and an aircraft-level assessment is performed
([`AIR-BAS-003`](requirements.md#air-bas-003),
[`AIR-HAZ-001`](requirements.md#air-haz-001)). Closure requires the qualified
independent safety review recorded at the end of this document; those fields
remain `PENDING`.

## Architecture under assessment

The assessed architecture is the AIR-01 [system boundary](system-boundary.md)
realized by [ADR-0017](../adr/0017-instrument-display-runtime.md) and
[ADR-0018](../adr/0018-avionics-telemetry-and-aviate-adapter.md):

- Untrusted adapters and transports deliver data to an **input validation and
  time/integrity gate**, which produces one atomic `DisplaySnapshot` with a
  coherence result and a wrapping generation. Transport delivery is not evidence
  of freshness or trust.
- A **no_std sans-IO core** derives display quantities and per-signal validity,
  emitting a versioned scene-command IR.
- The IR is partitioned into six ascending, unnested **criticality bands**
  (`Background < Attitude < Tapes < Guidance < Annunciation < Failure`); a
  **compositor** enforces that ordering so optional background can never cover,
  suppress, or replace primary symbology, warnings, or failure indications.
- A **deterministic output backend** renders the composed frame, and an
  **independent progress/output monitor** detects a renderer or output path that
  retains a last-good image and forces a display-failure presentation within an
  allocated time.

The FRAME-01 contract (issue #52) adds typed frame, epoch, clock-domain, and
time-scale identities to every presented spatial quantity, with checked
composition and fail-closed unknown/mismatch handling. This PSSA treats that
contract as a common resource whose integrity is that of the most critical
spatial presentation it feeds.

## Common-cause boundaries

Assurance is allocated by architecture rather than by crate because the failure
conditions in the [FHA](fha.md) — especially FC-CC-01 through FC-CC-04, FC-SRC-02,
and FC-MON-02 — are dominated by shared resources. Each boundary below is a
candidate common-cause group. Independence across a boundary is **assumed nowhere
today**; it must be established by a selected installation and its own analysis
([`AIR-HAZ-009`](requirements.md#air-haz-009)).

| Common-cause boundary | What is shared today | Independence required before credit | Simulator-only limitation |
|---|---|---|---|
| Sensors / source estimators | Single upstream estimate (e.g. one Aviate estimator) feeds all functions | Independent sensing/estimation channels for any comparison or redundancy credit | Simulated truth carries no sensor-error, installation, or integrity claim |
| Buses / transports | One WebTransport/shared-memory/USB CDC path | Physically and electrically independent buses for redundant channels | Receipt time is not source time; a transport cannot refresh age or integrity |
| Timebase | One acquisition/clock domain drives freshness and coherence (FC-CC-01) | Independent timebase for any function credited to survive a clock fault | Browser/simulator clocks are not a qualified time source |
| Source selection | One selection/authorization policy gates all groups | Selection logic independent from the sources it arbitrates | Simulator profile admits unseen incarnations and resets on session renegotiation; explicitly ineligible for credit |
| Databases | Shared terrain/obstacle/aerodrome data and one position source (FC-CC-04) | Independent provenance and position integrity for SVS versus primary cues | Database graphics are not sensor observations, TAWS, or assurance evidence |
| Renderer | One rasterizer/backend for every band | A reversion path whose renderer is independent of the primary path | Canvas/browser rendering is nondeterministic across platforms |
| Compositor / window manager | One compositor orders every criticality band (FC-CMP-01–03) | Compositor integrity at least that of the most critical band it can affect | Browser window manager and lifecycle are not a qualified compositor |
| Display hardware | One surface presents all functions (FC-CC-03) | Independent surfaces for any dual-display or standby credit | Browser display is not a qualified display platform |
| Power | One power domain for core, renderer, compositor, monitor (FC-CC-03) | Independent power for any independently-credited path | Not modelled by the simulator |
| Monitoring | One progress/output monitor (FC-MON-01–03) | Monitor independent of the computation, timebase, and power it checks | Simulator monitors demonstrate mechanism, not airborne coverage |

The single largest common-cause item is the **shared validated state copy**: the
no_std panels are pure functions of one immutable `DisplaySnapshot`, so drawing a
second instrument, a six-pack, or a "standby" from that same copy establishes no
independence (FC-CC-02, FC-INST-01). This is stated normatively by
[`AIR-HAZ-009`](requirements.md#air-haz-009).

## Independence assumptions (explicit)

Recorded so the [FHA](fha.md) classifications are not read as assuming redundancy
that does not exist:

- **No source independence.** A single upstream estimate feeds every function;
  misleading-source failure conditions (FC-ATT-02, FC-AD-02, FC-SRC-01) have no
  on-display detection without an independent comparison source.
- **No renderer, compositor, display, or power independence.** One process, one
  backend, one surface, one power domain; the reversion surface is not independent
  of the primary path (FC-CC-03).
- **No monitor independence yet.** The progress/output monitor shares the process
  and timebase of the path it monitors; it demonstrates the mechanism, not
  airborne coverage (FC-MON-02), pending
  [`AIR-HAZ-003`](requirements.md#air-haz-003).
- **Ingress identity is simulator-grade.** Per ADR-0018, the shared-memory reader
  uses POSIX `(device, inode, size)` identity and the browser profile uses
  operating-system entropy for incarnation; both are explicitly ineligible for
  operational credit and an aircraft producer must supply a source-issued boot
  identity.
- **Coherence is not redundancy.** The coherence result confirms that groups share
  identity, epoch, and clock within a skew bound; it is not a cross-source
  comparison and grants no miscompare detection.

## Assurance allocation rationale

No DAL/IDAL is assigned ([`AIR-BAS-003`](requirements.md#air-bas-003),
[`AIR-HAZ-001`](requirements.md#air-haz-001)). The allocation below is a
**relative, conditional** rationale: it says where integrity and independence
must concentrate *if* a basis credits these functions, and recommends independence
only where the intended function and architecture support it. Quantitative
objectives are deliberately absent because there is no defined certification basis
(that basis is the subject of AIR-03, issue #28).

| Function / element | Worst-credible-if-credited | Allocation rationale (conditional) |
|---|---|---|
| Attitude / PFD primary symbology | Catastrophic | Highest integrity; a credited basis would drive attitude-source independence, cross-comparison, and a genuinely independent reversion path. Misleading and frozen attitude (FC-ATT-02, FC-ATT-03) are the design drivers |
| Air data / altitude | Catastrophic | High integrity on value correctness, datum, and polarity; wrong-datum and substitution (FC-AD-03, FC-AD-06) require independent detection or unavailability |
| Heading / navigation / HSI | Hazardous | Integrity on reference typing (magnetic/true), deviation polarity, and navigation-mode declaration (FC-HDG-02, FC-HDG-03) |
| Conventional instruments | Inherits the quantity shown | No independent standby credit; integrity is that of the shared state and the honest-missing rule (FC-INST-01, FC-INST-02) |
| SVS background | Hazardous (via misleading terrain) or contained | Imagery fidelity is a lower tier, but **containment** — compositor priority and reversion — inherits the primary-path integrity, because an SVS fault must never touch symbology (FC-SVS-03, FC-SVS-05) |
| HUD-SIM / repeater | Hazardous | Integrity concentrated on calibration/time validation and on the fail-down to non-conformal (FC-HUD-01, FC-HUD-04); the conformal claim is the hazard |
| Frame / six-DoF contract (FRAME-01) | Catastrophic | A shared spatial resource; its integrity equals that of the most critical spatial presentation. Fail-closed on frame/epoch/clock/time-scale mismatch is mandatory (FC-FRM-01–05) |
| Compositor / window manager | Catastrophic | Inherits the highest criticality band it can affect; must fail toward exposing critical bands (FC-CMP-01–03) |
| Timebase | Catastrophic | Common-cause to every time-dependent function; independence and self-test bound its latent failure (FC-CC-01, FC-MON-03) |
| Progress / output monitor | Catastrophic | Must be independent of and bound the latency of the path it protects (FC-MON-01–03) |

The recurring PSSA conclusion is that **shared resources — the timebase, source
selection, compositor, renderer, monitor, and frame contract — inherit the
integrity of the most critical function they can affect**, and are therefore the
primary targets for independence and latent-failure control, not the individual
panels.

## Simulator versus airborne mitigations

Per the [FHA](fha.md) and issue #27, the analysis distinguishes what the simulator
demonstrates from what an airborne installation must provide.

| Failure condition family | Simulator mechanism (demonstrated) | Airborne mitigation (required, not provided here) |
|---|---|---|
| Misleading value (FC-ATT-02, FC-AD-02, FC-SRC-01) | Per-signal validation, range/coherence checks, miscompare when a second source exists | Independent sensing channels and a comparison/voting architecture |
| Frozen / stalled display (FC-ATT-03, FC-RND-01, FC-MON-01) | Progress/output monitor forces display-failure in the same process | An independent hardware monitor and independent power |
| Wrong datum / reference / frame (FC-AD-03, FC-HDG-02, FC-FRM-01–05) | Declared metadata, fail-closed on missing/unknown, typed frames | Qualified source metadata and installation-verified references |
| Compositor priority error (FC-CMP-01–03) | Validated ascending layer encoding, unknown-layer-id frame failure, budgets | Qualified compositor/window manager with resource guarantees |
| Timebase / replay (FC-TIME-01–02, FC-CC-01) | Epoch, sequence, time-domain mapping, coherence result | Independent qualified timebase; source-issued boot identity |
| Common-cause (FC-CC-01–04, FC-SRC-02, FC-MON-02) | Boundary declaration and containment rules | Physical/electrical/power separation and a documented common-cause analysis |

## Traceability

Traceability is bidirectional through the requirement registry: implementation and
verification issues cite intended-function requirements under change control
([`AIR-BAS-005`](requirements.md#air-bas-005)), and this assessment cites those
issues back from each failure condition and derived requirement. All identifiers
in the tables below link into the registry; issue references are GitHub issues in
`luofang34/Pilotage`.

### Failure-condition family → derived requirement → issue

| Failure-condition family | Primary derived requirement(s) | Implementation / verification issue(s) |
|---|---|---|
| Attitude (FC-ATT-01–07) | [`AIR-UNAV-001`](requirements.md#air-unav-001), [`AIR-HAZ-002`](requirements.md#air-haz-002), [`AIR-HAZ-011`](requirements.md#air-haz-011), [`AIR-HAZ-012`](requirements.md#air-haz-012) | [#17](https://github.com/luofang34/Pilotage/issues/17), [#19](https://github.com/luofang34/Pilotage/issues/19), [#20](https://github.com/luofang34/Pilotage/issues/20), [#15](https://github.com/luofang34/Pilotage/issues/15) |
| Air data (FC-AD-01–06) | [`AIR-UNAV-002`](requirements.md#air-unav-002), [`AIR-HAZ-002`](requirements.md#air-haz-002), [`AIR-HAZ-007`](requirements.md#air-haz-007), [`AIR-HAZ-012`](requirements.md#air-haz-012) | [#16](https://github.com/luofang34/Pilotage/issues/16), [#19](https://github.com/luofang34/Pilotage/issues/19), [#20](https://github.com/luofang34/Pilotage/issues/20) |
| Heading / navigation (FC-HDG-01–05) | [`AIR-UNAV-003`](requirements.md#air-unav-003), [`AIR-HAZ-007`](requirements.md#air-haz-007), [`AIR-HAZ-012`](requirements.md#air-haz-012) | [#21](https://github.com/luofang34/Pilotage/issues/21), [#20](https://github.com/luofang34/Pilotage/issues/20), [#23](https://github.com/luofang34/Pilotage/issues/23) |
| Conventional instruments (FC-INST-01–02) | [`AIR-OUT-003`](requirements.md#air-out-003), [`AIR-HAZ-002`](requirements.md#air-haz-002), [`AIR-HAZ-009`](requirements.md#air-haz-009) | [#20](https://github.com/luofang34/Pilotage/issues/20) |
| Synthetic vision (FC-SVS-01–06) | [`AIR-OUT-005`](requirements.md#air-out-005), [`AIR-OUT-010`](requirements.md#air-out-010), [`AIR-UNAV-005`](requirements.md#air-unav-005), [`AIR-HAZ-004`](requirements.md#air-haz-004), [`AIR-HAZ-006`](requirements.md#air-haz-006) | [#34](https://github.com/luofang34/Pilotage/issues/34), [#33](https://github.com/luofang34/Pilotage/issues/33), [#31](https://github.com/luofang34/Pilotage/issues/31), [#32](https://github.com/luofang34/Pilotage/issues/32), [#35](https://github.com/luofang34/Pilotage/issues/35) |
| HUD-SIM / repeater (FC-HUD-01–04) | [`AIR-OUT-006`](requirements.md#air-out-006), [`AIR-OUT-007`](requirements.md#air-out-007), [`AIR-UNAV-006`](requirements.md#air-unav-006), [`AIR-HAZ-005`](requirements.md#air-haz-005), [`AIR-HAZ-011`](requirements.md#air-haz-011) | [#36](https://github.com/luofang34/Pilotage/issues/36), [#37](https://github.com/luofang34/Pilotage/issues/37), [#38](https://github.com/luofang34/Pilotage/issues/38), [#39](https://github.com/luofang34/Pilotage/issues/39) |
| Frame / six-DoF (FC-FRM-01–05) | [`AIR-HAZ-006`](requirements.md#air-haz-006), [`AIR-HAZ-007`](requirements.md#air-haz-007), [`AIR-HAZ-008`](requirements.md#air-haz-008), [`AIR-UNAV-004`](requirements.md#air-unav-004) | [#52](https://github.com/luofang34/Pilotage/issues/52), [#18](https://github.com/luofang34/Pilotage/issues/18), [#46](https://github.com/luofang34/Pilotage/issues/46) |
| Timebase (FC-TIME-01–02) | [`AIR-IN-008`](requirements.md#air-in-008), [`AIR-UNAV-004`](requirements.md#air-unav-004), [`AIR-HAZ-011`](requirements.md#air-haz-011) | [#18](https://github.com/luofang34/Pilotage/issues/18), [#46](https://github.com/luofang34/Pilotage/issues/46) |
| Source selection (FC-SRC-01–02) | [`AIR-FLAG-006`](requirements.md#air-flag-006), [`AIR-HAZ-002`](requirements.md#air-haz-002), [`AIR-HAZ-009`](requirements.md#air-haz-009) | [#20](https://github.com/luofang34/Pilotage/issues/20) |
| Compositor (FC-CMP-01–03) | [`AIR-HAZ-004`](requirements.md#air-haz-004), [`AIR-BAS-007`](requirements.md#air-bas-007) | [#25](https://github.com/luofang34/Pilotage/issues/25), [#30](https://github.com/luofang34/Pilotage/issues/30) |
| Renderer / output (FC-RND-01–03) | [`AIR-UNAV-007`](requirements.md#air-unav-007), [`AIR-BAS-007`](requirements.md#air-bas-007), [`AIR-HAZ-011`](requirements.md#air-haz-011) | [#15](https://github.com/luofang34/Pilotage/issues/15), [#26](https://github.com/luofang34/Pilotage/issues/26), [#29](https://github.com/luofang34/Pilotage/issues/29), [#30](https://github.com/luofang34/Pilotage/issues/30) |
| Monitor (FC-MON-01–03) | [`AIR-IN-013`](requirements.md#air-in-013), [`AIR-HAZ-003`](requirements.md#air-haz-003), [`AIR-HAZ-010`](requirements.md#air-haz-010) | [#15](https://github.com/luofang34/Pilotage/issues/15), [#30](https://github.com/luofang34/Pilotage/issues/30) |
| Common-cause (FC-CC-01–04) | [`AIR-HAZ-009`](requirements.md#air-haz-009), [`AIR-HAZ-003`](requirements.md#air-haz-003), [`AIR-UNAV-004`](requirements.md#air-unav-004) | *see unallocated table* |

### Derived requirement → issue allocation

| Derived requirement | Allocated issue(s) | Verification intent |
|---|---|---|
| [`AIR-HAZ-001`](requirements.md#air-haz-001) | [#28](https://github.com/luofang34/Pilotage/issues/28) | Certification-basis and lifecycle plan keeps classifications conditional; review confirms no unconditional severity appears |
| [`AIR-HAZ-002`](requirements.md#air-haz-002) | [#19](https://github.com/luofang34/Pilotage/issues/19), [#20](https://github.com/luofang34/Pilotage/issues/20), [#15](https://github.com/luofang34/Pilotage/issues/15) | Per-signal validation + comparison/reversion; a quantity that cannot be shown correct is unavailable |
| [`AIR-HAZ-003`](requirements.md#air-haz-003) | [#15](https://github.com/luofang34/Pilotage/issues/15), [#30](https://github.com/luofang34/Pilotage/issues/30) | Monitor coverage and fault-injection; installation-level independence remains an aircraft-architecture gap |
| [`AIR-HAZ-004`](requirements.md#air-haz-004) | [#25](https://github.com/luofang34/Pilotage/issues/25), [#30](https://github.com/luofang34/Pilotage/issues/30) | Layer-contract validation and conformance/fault-injection gates |
| [`AIR-HAZ-005`](requirements.md#air-haz-005) | [#20](https://github.com/luofang34/Pilotage/issues/20), [#35](https://github.com/luofang34/Pilotage/issues/35), [#15](https://github.com/luofang34/Pilotage/issues/15) | Deterministic reversion, SVS fallback, and display-failure on reversion-mechanism fault |
| [`AIR-HAZ-006`](requirements.md#air-haz-006) | [#52](https://github.com/luofang34/Pilotage/issues/52), [#18](https://github.com/luofang34/Pilotage/issues/18), [#46](https://github.com/luofang34/Pilotage/issues/46) | Typed-frame checked composition; fail-closed on mismatch/unknown |
| [`AIR-HAZ-007`](requirements.md#air-haz-007) | [#52](https://github.com/luofang34/Pilotage/issues/52), [#16](https://github.com/luofang34/Pilotage/issues/16), [#21](https://github.com/luofang34/Pilotage/issues/21) | Explicit reference/datum/local-vertical basis; no fabricated horizon |
| [`AIR-HAZ-008`](requirements.md#air-haz-008) | [#52](https://github.com/luofang34/Pilotage/issues/52), [#34](https://github.com/luofang34/Pilotage/issues/34) | Distinct labelled projections; annunciated basis change |
| [`AIR-HAZ-009`](requirements.md#air-haz-009) | *unallocated* | Analysis obligation; see unallocated table |
| [`AIR-HAZ-010`](requirements.md#air-haz-010) | *unallocated* | Exposure intervals need a certification basis; see unallocated table |
| [`AIR-HAZ-011`](requirements.md#air-haz-011) | [#15](https://github.com/luofang34/Pilotage/issues/15), [#18](https://github.com/luofang34/Pilotage/issues/18) | Source-time freshness + renderer-progress liveness |
| [`AIR-HAZ-012`](requirements.md#air-haz-012) | [#17](https://github.com/luofang34/Pilotage/issues/17), [#23](https://github.com/luofang34/Pilotage/issues/23), [#18](https://github.com/luofang34/Pilotage/issues/18) | Polarity/ordering integrity through the orientation domain and reversion |

### Unallocated derived requirements

Requirements with no existing implementation or verification issue. Per issue #27
these are listed explicitly rather than assigned invented issue numbers; each
needs a new issue or a certification-basis decision.

| Derived requirement | Why unallocated | What is needed |
|---|---|---|
| [`AIR-HAZ-009`](requirements.md#air-haz-009) | Common-cause boundary declaration is an ongoing analysis obligation against a selected architecture; no implementation issue owns the power, bus, and display-hardware separation it demands | A dedicated common-cause / independence analysis issue tied to a selected installation, plus the aircraft power and separation architecture |
| [`AIR-HAZ-010`](requirements.md#air-haz-010) | Latent-failure exposure intervals cannot be set without a certification basis and its quantitative objectives; the monitor and reversion self-test cadence is undefined pre-basis | Exposure-interval derivation under the AIR-03 basis ([#28](https://github.com/luofang34/Pilotage/issues/28)) and a self-test/BIT implementation issue |

Additional unallocated safety gaps identified by the analysis, each requiring a
future issue: **independent power and physical separation** for the reversion path
(FC-CC-03); an **independent hardware monitor** for airborne coverage (FC-MON-02,
partially served by [#30](https://github.com/luofang34/Pilotage/issues/30) in
simulation only); and **independent sensing/comparison sources** for
misleading-value detection (FC-ATT-02, FC-AD-02), which are outside this boundary
and belong to an aircraft sensor architecture.

## Analysis snapshot (informative, non-normative)

The GitHub issue tracker is the authoritative source for the current state of
every referenced issue. As of this analysis revision, the implementation baseline
that already exists in the merged registry covers input validation, measurement
identity and coherence, fail-closed authorization, the scene-layer/compositor
contract, the reproducible glyph pack, the reference rasterizer, display-liveness
failure, and SO(3) unusual-attitude presentation. The synthetic-vision, HUD-SIM,
alerting, source-comparison, frame-contract, and certification-basis work is
tracked by open issues. This paragraph is informative context only; nothing in the
FHA/PSSA classifications depends on an issue's open/closed state.

## Independent safety review record (AIR-02)

This record is the closure gate for AIR-02, mirroring
[`AIR-BAS-006`](requirements.md#air-bas-006) and the AIR-01
[review record](review-record.md). Empty or `PENDING` fields mean this assessment
is not approved and issue #27 remains open. A pull-request approval without the
qualifications and disposition below is not a substitute.

### Assessment under review

- Tracking issue: AIR-02 / GitHub issue 27
- Artifacts: `fha.md`, `pssa.md`, and the `AIR-HAZ` requirements in
  `requirements.md` in this directory
- Baseline analysed: the AIR-01 intended-function baseline and the FRAME-01
  (issue 52) frame contract
- Certification or operational approval: not asserted by this review

### Safety review

- Reviewer: PENDING
- Qualification and relevant experience: PENDING
- Independence from the author: PENDING
- Review revision/commit: PENDING
- Date: PENDING
- Disposition (`APPROVED`, `APPROVED WITH ACTIONS`, or `REJECTED`): PENDING
- Open actions and linked issues: PENDING
- Recorded rationale: PENDING

The safety reviewer confirms that the failure-condition set is adequately
complete for the analysed functions, that severities are stated conditionally with
no unconditional classification or DAL, that common-cause boundaries and
independence assumptions are explicit, that simulator mechanisms are distinguished
from airborne mitigations, and that every derived requirement is either allocated
to an issue or listed as unallocated.

### Human-factors review

- Reviewer: PENDING
- Qualification and relevant experience: PENDING
- Independence from the author: PENDING
- Review revision/commit: PENDING
- Date: PENDING
- Disposition (`APPROVED`, `APPROVED WITH ACTIONS`, or `REJECTED`): PENDING
- Open actions and linked issues: PENDING
- Recorded rationale: PENDING

The human-factors reviewer confirms that the crew-effect descriptions, detection
assumptions, reversion behavior, and mode/annunciation failure conditions are
suitable inputs to subsequent aircraft-specific validation.

### Closure decision

- All required reviews complete: NO
- Certification basis selected and classifications made unconditional under it: NO
- All blocking actions resolved or accepted by the responsible authority: NO
- Tracking issue may close: NO
- Decision revision/commit: PENDING
- Decision owner and date: PENDING
