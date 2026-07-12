# Lifecycle evidence plan (AIR-03)

**SIM / NOT FOR FLIGHT.** This document plans the lifecycle evidence a future
Pilotage instrument certification effort would need, indexes the artifacts that
exist in the repository today, and states plainly which of those artifacts are
**engineering input** versus which certified lifecycle data must be established
anew. It is **not** a compliance record, and it does not claim that any existing
artifact satisfies a certification objective. Existing prototype history is never
represented here as certified lifecycle data.

## Purpose

A credible certification effort is evidence-driven: each objective is satisfied
by a named, configuration-controlled artifact. This plan does two honest things.
First, it inventories the requirements, design, code, test, review, and
configuration material that already exists, with pointers to the actual files.
Second, it draws a hard line between that prototype material — usable as
engineering input — and the DO-178C lifecycle data that must be produced under an
allocated software level and cannot be back-claimed from prototype history.

The software levels that scale most DO-178C objectives are **not yet allocated**.
AIR-02 performs that allocation and is preliminary and pending review (issue #27,
review record `PENDING`). Wherever an objective depends on an allocated level,
this plan marks it **deferred**, not satisfied.

## Baselines that exist today

These are real, tracked artifacts. Their existence is a statement of engineering
work performed, not of certification credit earned.

### Requirements baseline

- [`requirements.md`](requirements.md) — the stable `AIR-*` intended-function and
  hazard-derived requirement registry, guarded for identifier integrity and link
  correctness by `scripts/check-instrument-requirements.sh`.
- [`intended-functions.md`](intended-functions.md) — per-function intended
  behavior, modes, and source assumptions.
- [`system-boundary.md`](system-boundary.md) — trusted/untrusted interfaces,
  simulator-only components, and reversion paths.

### Design baseline

- `docs/adr/0001`–`0019` — the architecture decision record set, including
  ADR-0015 (workspace quality gates), ADR-0017 (instrument display runtime),
  ADR-0018 (avionics telemetry and Aviate adapter), and ADR-0019 (pluggable
  vehicle link).
- [`scene-layer-protocol.md`](scene-layer-protocol.md) and
  [`glyph-pack.md`](glyph-pack.md) — the controlled display-content contracts.
- `docs/architecture.md` — the system-level architecture description.

### Code baseline

- The `pilotage-instrument-*` crate family (`-state`, `-scene`, `-glyphs`,
  `-panels`, `-raster`) plus `pilotage-frames` and `pilotage-alerts`, compiled
  for a bare-metal target in CI to substantiate the `no_std` claim (ADR-0017).
- `clients/web-instruments` — the WebAssembly instrument backend.
- `clients/web` — the browser viewer and its wire/telemetry/transport paths.

### Test baseline

- The in-crate `tests.rs` and `tests/` modules across the workspace (unit and
  property tests with direct access to internals or public-API integration
  tests).
- The reference-rasterizer frame-hash tests (`pilotage-instrument-raster`) that
  pin PFD/HSI output to SHA-256 hashes for bit-reproducibility (REN-03).
- The glyph-pack integrity tests (`pilotage-instrument-glyphs`) for vocabulary
  completeness and fail-closed corruption detection (REN-02).
- The browser conformance suites run in CI: `wire.test.mjs`,
  `telemetry-ingress.test.mjs`, `telemetry-display.test.mjs`,
  `transport-session.test.mjs`, `instruments.test.mjs`, and
  `scene-conformance.test.mjs`.
- [`renderer-verification.md`](renderer-verification.md) — the verification wall
  defining what each rendering backend must prove, its budgets, and the shared
  conformance corpus.

### Review baseline

- [`review-record.md`](review-record.md) — the AIR-01 closure gate. Every
  reviewer field currently reads `PENDING`; the baseline is **not approved** and
  issue #24 remains open ([`AIR-BAS-006`](requirements.md#air-bas-006)).
- The AIR-02 review record embedded in [`pssa.md`](pssa.md) — likewise `PENDING`;
  issue #27 remains open.

### Configuration baseline

- The git commit history and the pull-request record are the configuration record
  of engineering work (for example the merged increment and flight-mode PRs
  visible in history).
- ADR-0015 workspace quality gates, `.github/workflows/ci.yml`,
  `scripts/check-structure.sh`, `scripts/check-instrument-requirements.sh`, and
  `scripts/structure-function-baseline.tsv` are the automated configuration and
  structural controls in force today.

## Evidence index

Each row maps an objective family to the artifact(s) that would carry it and the
honest current status. `deferred` means the objective depends on an allocation or
selection not yet made. `engineering input` means an artifact exists but is not
certified lifecycle data. Nothing here is marked satisfied.

| ID | Objective family (standard) | Current artifact(s) | Status |
| --- | --- | --- | --- |
| EVP-01 | System/development-assurance planning (ARP4754A) | `requirements.md`, ADR set, `docs/architecture.md` | engineering input; process not exercised to closure |
| EVP-02 | Safety assessment (ARP4761 / FHA/PSSA) | [`fha.md`](fha.md), [`pssa.md`](pssa.md) | engineering input; **preliminary**, issue #27 `PENDING` |
| EVP-03 | Software planning (DO-178C plans: PSAC/SDP/SVP/SCMP/SQAP) | none yet | deferred until level allocation (AIR-02, #27) |
| EVP-04 | Software requirements (DO-178C) | `requirements.md` (`AIR-*`), intended-functions | engineering input; not level-allocated certified requirements |
| EVP-05 | Software design (DO-178C) | ADR-0017/0018/0019, scene-layer/glyph-pack contracts | engineering input |
| EVP-06 | Software coding (DO-178C) | `pilotage-instrument-*`, `pilotage-frames`, `pilotage-alerts`, `clients/web-instruments` | engineering input; **not** certified source data |
| EVP-07 | Verification — reviews and analyses (DO-178C) | code review in PR history, `renderer-verification.md` | engineering input; not DO-178C-conformant review records |
| EVP-08 | Verification — testing (DO-178C) | workspace `tests.rs`/`tests/`, browser `*.test.mjs`, raster/glyph tests | engineering input; not requirements-based certified test evidence |
| EVP-09 | Structural coverage (DO-178C) | none | **deferred** — see structural-coverage section |
| EVP-10 | Configuration management (DO-178C SCM) | git/PR history, ADR-0015 gates, CI, structure checks | engineering configuration record; not certified SCM data |
| EVP-11 | Quality assurance (DO-178C SQA) | CI quality gates | engineering practice; independent SQA function not established |
| EVP-12 | Problem reporting (DO-178C) | GitHub issues and PRs | engineering record; not certified problem-report data |
| EVP-13 | Tool qualification (DO-330) | none | deferred; per-tool TQL undetermined until level allocation |
| EVP-14 | Complex hardware (DO-254) | none | not applicable (no custom complex AEH in scope) |
| EVP-15 | Environmental qualification (DO-160G) | none | not applicable (no airborne equipment); DO-160H excluded |
| EVP-16 | Aeronautical data (DO-200B / ED-76) | none (simulator/reference data only) | not applicable yet (no operational data chain) |
| EVP-17 | Security (DO-326A / ED-202A airworthiness-security family) | [`system-boundary.md`](system-boundary.md) trust analysis | engineering input; not a security certification artifact — see [standards matrix](standards-applicability.md) STD-050..STD-052 |
| EVP-18 | Display / alerting guidance (AC 25-11B; alert model) | PFD/HSI intended functions, `pilotage-alerts` crate | engineering input; no display submitted for approval |
| EVP-19 | Installation and flight-test evidence | none | not applicable (no installation; [`AIR-OUT-008`](requirements.md#air-out-008)) |
| EVP-20 | Synthetic-vision performance (DO-407 / ED-326 and earlier DO-315 / ED-179; AC 20-167A / AC 20-185) | SVS is supplemental only ([`AIR-OUT-005`](requirements.md#air-out-005)); no operational-credit artifact | engineering input; DO-407 / ED-326 released MASPS with authority acceptance unresolved — see [standards matrix](standards-applicability.md) STD-066 |
| EVP-21 | Source equipment — attitude/heading (AC 20-181, AHRS / TSO-C201) | attitude and heading inputs ([`AIR-IN-001`](requirements.md#air-in-001), [`AIR-IN-005`](requirements.md#air-in-005)) | not applicable yet (no airborne AHRS source selected) — see [standards matrix](standards-applicability.md) STD-065 |

## Reuse as engineering input versus evidence established anew

This section is the anti-back-claim boundary. It is deliberately explicit.

**May be reused as engineering input** (informs the certified effort, cited as
prior engineering, never relabeled as certified data):

- The `AIR-*` requirement registry as a starting requirement set.
- The FHA/PSSA analysis as a starting safety analysis (preliminary; #27 `PENDING`).
- The architecture decision records and display-content contracts as design
  rationale.
- The prototype source and tests as reference implementations and as evidence
  that a behavior is achievable.
- The CI gates, structural checks, and conformance corpora as engineering
  verification practice and as the seed for a certified verification environment.

**Must be established anew under an allocated software level** (cannot be
back-claimed from prototype history):

- DO-178C plans (PSAC, SDP, SVP, SCMP, SQAP) and their authority agreement.
- Requirements, design, and code baselined and traced **as certified lifecycle
  data**, at the level AIR-02 allocates, with DO-178C-conformant review records.
- Requirements-based test cases and procedures with traceability, plus the
  structural coverage appropriate to the allocated level.
- Tool-qualification data (DO-330) for any tool whose output is relied upon
  without independent verification.
- Configuration management and problem-reporting records under a
  certification-conformant SCM/SQA process, distinct from the current git/PR/CI
  record.

**Explicit non-back-claim statements:**

- Passing CI and passing tests are engineering signals; they are **not** DO-178C
  verification credit and are not represented as such.
- The git and pull-request history is a configuration record of **engineering
  work**; it is **not** certified configuration data, and its existence does not
  establish DO-178C SCM objective satisfaction.
- Preliminary FHA/PSSA classifications remain conditional and unclosed
  ([`AIR-HAZ-001`](requirements.md#air-haz-001)); they confer no assurance level.

## Structural coverage — deferred

DO-178C structural-coverage objectives (statement, decision, and modified
condition/decision coverage) scale with software level. **No software level is
allocated**: that allocation is AIR-02's responsibility and AIR-02 is preliminary
and pending review (issue #27, review record `PENDING`). Consequently:

- Structural coverage is **deferred**. It is not claimed, and it is not planned to
  a specific coverage criterion, because the governing level does not exist yet.
- The existing frame-hash and conformance tests demonstrate reproducibility and
  behavior; they are **not** a structural-coverage measurement and are not
  represented as one.
- When AIR-02 allocates levels, this section is replaced by a coverage objective
  and a measurement method appropriate to each allocated level, and the
  latent-failure exposure intervals ([`AIR-HAZ-010`](requirements.md#air-haz-010))
  are set against the selected certification basis.

## Acceptance-criteria coverage

The AIR-03 acceptance criteria require the evidence plan to cover a specific set
of standards families. Each is addressed above:

- ARP4754 / ARP4761 — EVP-01, EVP-02.
- DO-178C — EVP-03 through EVP-12 and the structural-coverage section.
- DO-254 — EVP-14 (not applicable, current scope).
- DO-160 — EVP-15 (not applicable; DO-160G recorded, DO-160H excluded).
- Aeronautical data — EVP-16.
- Security (DO-326A / ED-202A airworthiness-security family) — EVP-17 and
  [standards matrix](standards-applicability.md) STD-050..STD-052.
- Synthetic-vision performance, including DO-407 / ED-326 (the SVS/SVGS/CVS MASPS,
  not a security standard) — EVP-20 and [standards matrix](standards-applicability.md)
  STD-066.
- Display / alerting guidance — EVP-18; attitude/heading source equipment
  (AC 20-181, AHRS) — EVP-21 and STD-065.
- Installation qualification — EVP-19.

## Re-verification clause

This plan records status as of 2026-07-12 and is provisional. The evidence index
must be re-baselined once AIR-02 allocates software levels and a certification
authority, aircraft, operation, installation, and certification basis are
selected. Until then it is an engineering planning input and confers no
certification credit.
