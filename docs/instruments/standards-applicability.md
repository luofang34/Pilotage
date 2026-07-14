# Standards applicability matrix (AIR-03)

**SIM / NOT FOR FLIGHT.** This document classifies the industry standards,
consensus references, and authority guidance that a future Pilotage instrument
certification effort would draw on. It is an engineering planning artifact. It
is **not** a compliance finding, a declaration that Pilotage meets any standard,
an FAA/EASA finding, a TSO authorization, or a TC/STC approval. Nothing in this
matrix authorizes airborne use of any Pilotage output.

## Purpose

The matrix separates *what current engineering practice would use* from *what a
selected certification authority has formally recognized*. Those two are not the
same: the newest revision of an industry standard is frequently ahead of the
revision named in the authority guidance that would credit it. Recording the gap
honestly — rather than quietly adopting the newest revision and implying it is
accepted — is the entire point of this artifact.

For each standard the matrix records: the selected revision, an authority-status
classification, the rationale, known gaps, and whether an issue paper or explicit
authority agreement would be required before the revision could be used as a
certification basis.

## Status of this matrix

- This matrix records status **as of the plan date, 2026-07-12**. Standard
  revisions, advisory-circular recognitions, and RTCA/EUROCAE publication states
  change over time; every row **must be re-verified with the selected
  certification authority at the point of authority engagement**. The matrix is a
  planning input, not a standing statement of current recognition.
- **Standard identities were verified against publisher listings on 2026-07-12**
  (RTCA, EUROCAE, and FAA advisory-circular listings; per-row source links in the
  matrix). That verification fixes each document's identity, title, and released
  status as of that date. It does **not** establish authority acceptance:
  authority-acceptance status is recorded separately per row and must be
  re-verified with the selected authority at authority engagement.
- No target aircraft, operating rule, certification authority, certification
  basis, or installed equipment has been selected. Applicability that depends on
  those selections is recorded as conditional, not resolved.
- This matrix builds on the AIR-01 intended-function baseline and the AIR-02
  preliminary safety assessment. **Both are preliminary and pending independent
  human review**: AIR-01 review is tracked by issue #24 and AIR-02 review by
  issue #27, and both review records currently read `PENDING`
  ([`AIR-BAS-006`](requirements.md#air-bas-006)). Assurance-level-dependent
  applicability in this matrix is therefore provisional until AIR-02 closes.

## Structured registry and drift guard

The vision-guidance references whose active revision and supersession state are
easiest to get wrong — the FAA vision advisory circulars and the harmonized
synthetic-vision MASPS — have a single machine-checkable source of truth in
[`standards-registry.toml`](standards-registry.toml). For each it records the
reference identity, the selected (active) revision, the publisher URL, the
authority status, what the active revision supersedes, and a `verified_on`
date. The registry rows below (STD-061, STD-062, STD-066) are **checked against**
that file, not maintained independently of it.

`scripts/check-standards-registry.sh` fails CI when the registry is missing or
empty (fail-closed — a data outage is never green), when an entry lacks status
provenance (identity, authority status, publisher URL, or a well-formed
`verified_on`), when it classifies a revision as both active and superseded, or
when this matrix's revision or authority-status cells diverge from the registry.
The guard checks internal consistency and matrix/registry agreement only. It
does **not** re-fetch any source and does not prove a revision is still current:
external freshness remains a periodic expert and source review and is never
presented as automatically proven by CI.

## Authority-status vocabulary

Each row is classified as exactly one of the following for its *selected*
revision. Where a newer industry revision exists, its status is recorded in the
rationale and gap columns rather than by silently selecting it.

| Status | Meaning |
| --- | --- |
| `authority-accepted` | The selected revision is the one recognized by active guidance from the anticipated authority (e.g. an FAA Advisory Circular that names it), so it can anchor a certification basis without a separate agreement. |
| `latest engineering baseline` | The selected revision is the current industry revision and is used as engineering practice, but it is newer than the revision named in active authority guidance. |
| `requires authority agreement` | The revision cannot be used as a certification basis until the selected authority agrees, typically through an issue paper or means-of-compliance record. |
| `not applicable` | The standard governs a product element that is out of the current SIM-only scope; it is recorded so its exclusion is explicit and revisited if scope changes. |

When a standard has a recognized anchor revision *and* a newer industry revision,
the row selects the **anchor** revision as the status-bearing choice and records
the newer revision as a tracked `requires authority agreement` item. This keeps
the "selected revision" column honest: it names what could anchor a basis today,
not the newest document available.

## Matrix

### Systems and safety

| ID | Standard / reference | Selected revision | Authority status | Rationale | Known gaps | Issue paper / authority agreement |
| --- | --- | --- | --- | --- | --- | --- |
| STD-001 | SAE ARP4754 — development of civil aircraft and systems | ARP4754A (anchor); ARP4754B tracked | `authority-accepted` (A) | Active FAA AC 20-174 explicitly recognizes ARP4754A, so ARP4754A anchors the development-assurance process. ARP4754B is the current industry revision (a `latest engineering baseline`) but is ahead of the AC 20-174 recognition. | No aircraft/system selected, so development-assurance planning is provisional. Crediting ARP4754B is not yet supported by recognized guidance. | Issue paper / authority agreement **required** to credit ARP4754B in place of the AC 20-174-recognized ARP4754A. |
| STD-002 | SAE ARP4761 — safety assessment process | ARP4761 (anchor); ARP4761A tracked | `authority-accepted` (original) | The safety-assessment guidance recognized alongside ARP4754A is the original ARP4761. ARP4761A is the current industry revision (a `latest engineering baseline`) and is newer than the recognized revision. | The AIR-02 FHA/PSSA are preliminary (issue #27 `PENDING`); the safety process is not exercised to closure. | Issue paper / authority agreement **required** to credit ARP4761A. |
| STD-003 | Aircraft-level system safety objectives (25.1309 practice) | AC 25.1309-1B (guidance) | `authority-accepted` (guidance) | AC 25.1309-1B is recognized FAA guidance for system design and analysis and is the design reference used by AIR-02. It is applied as an engineering input, not as a satisfied objective. | No selected aircraft class or certification basis; the 25.1309 objective set is not allocated ([`AIR-HAZ-001`](requirements.md#air-haz-001)). | Authority agreement on the applicable regulatory paragraph follows aircraft selection. |

### Software

| ID | Standard / reference | Selected revision | Authority status | Rationale | Known gaps | Issue paper / authority agreement |
| --- | --- | --- | --- | --- | --- | --- |
| STD-010 | RTCA DO-178C — software considerations in airborne systems | DO-178C | `authority-accepted` | Active FAA AC 20-115D recognizes DO-178C as an accepted means of compliance for airborne software. It is the software-assurance anchor. | No software level is allocated; the assurance objectives that DO-178C scales by level are **deferred** until AIR-02 allocates levels (issue #27 `PENDING`). Prototype code is engineering input, not DO-178C lifecycle data (see [evidence plan](evidence-plan.md)). | None to adopt DO-178C itself; objective applicability follows the AIR-02 level allocation. |
| STD-011 | RTCA DO-330 — software tool qualification considerations | DO-330 | `authority-accepted` | DO-330 is the tool-qualification framework referenced by DO-178C and recognized through AC 20-115D. Applicable when a tool's output is relied on without independent verification. | The set of tools requiring qualification, and their Tool Qualification Levels, cannot be fixed until software levels and the verification approach are set (AIR-02, issue #27 `PENDING`). | None to adopt DO-330; per-tool TQL determinations follow level allocation. |
| STD-012 | RTCA DO-331 — model-based development and verification supplement | DO-331 | `authority-accepted` when invoked | Recognized supplement to DO-178C; applicable **only if** model-based development or verification is used. | Applicability undetermined: the development method for a future certified build is not selected. | None beyond the DO-178C basis; applies only if MBD is adopted. |
| STD-013 | RTCA DO-332 — object-oriented technology and related techniques supplement | DO-332 | `authority-accepted` when invoked | Recognized supplement to DO-178C; applicable **only if** object-oriented or related techniques are used in certified software. | Applicability undetermined; the current prototype is not the certified architecture. | None beyond the DO-178C basis; applies only if OOT is adopted. |
| STD-014 | RTCA DO-333 — formal methods supplement | DO-333 | `authority-accepted` when invoked | Recognized supplement to DO-178C; applicable **only if** formal methods are credited toward objectives. | Applicability undetermined; no formal-methods credit is claimed. | None beyond the DO-178C basis; applies only if formal-methods credit is sought. |

### Complex hardware

| ID | Standard / reference | Selected revision | Authority status | Rationale | Known gaps | Issue paper / authority agreement |
| --- | --- | --- | --- | --- | --- | --- |
| STD-020 | RTCA DO-254 — design assurance for airborne electronic hardware | DO-254 (revision recognized by AC 20-152A) | `not applicable` (current scope) | No custom or complex airborne electronic hardware is developed in the SIM-only program; there is no airborne display computer, FPGA, or ASIC in scope. AC 20-152A recognizes DO-254 for when such hardware exists. | A future installation that develops complex custom AEH (e.g. a display processor) would make DO-254 applicable and require a hardware-assurance plan. | Determined at hardware selection; not applicable until custom complex AEH enters scope. |

### Environmental qualification

| ID | Standard / reference | Selected revision | Authority status | Rationale | Known gaps | Issue paper / authority agreement |
| --- | --- | --- | --- | --- | --- | --- |
| STD-030 | RTCA DO-160 — environmental conditions and test procedures | DO-160G (current released revision) | `not applicable` (current scope) | Environmental qualification applies to installed airborne equipment, of which the SIM-only program has none. RTCA currently lists DO-160G as the current released revision; it is recorded as the revision to use **when** equipment is selected. **DO-160H is in work and is not listed here as an applicable released standard** — it must not be cited as a basis until it is published and the authority has dispositioned it. | No hardware to qualify; environmental categories cannot be assigned without a selected installation. | Determined at equipment selection; DO-160H excluded until published and dispositioned. |

### Aeronautical data

| ID | Standard / reference | Selected revision | Authority status | Rationale | Known gaps | Issue paper / authority agreement |
| --- | --- | --- | --- | --- | --- | --- |
| STD-040 | RTCA DO-200 / EUROCAE ED-76 — processing of aeronautical data | DO-200B / ED-76 (anchor); DO-200C / ED-76B tracked | `authority-accepted` (anchor) | Active FAA AC 20-153B recognizes the aeronautical-data revision it names as the accepted process. DO-200C / ED-76B is a current engineering baseline (a `latest engineering baseline`) and is newer than the AC 20-153B recognition. The synthetic-vision data chain ([`AIR-IN-011`](requirements.md#air-in-011)) would invoke this standard once an operational data supplier is used. | The program processes only simulator/reference data today; there is no operational aeronautical-data supply chain, so applicability is conditional. | Issue paper / authority agreement **required** to credit DO-200C / ED-76B in place of the AC 20-153B-recognized revision. |

### Security

| ID | Standard / reference | Selected revision | Authority status | Rationale | Known gaps | Issue paper / authority agreement |
| --- | --- | --- | --- | --- | --- | --- |
| STD-050 | Airworthiness security process (DO-326A / ED-202A) | DO-326A / ED-202A | `not applicable` (current scope) as certification obligation | The airworthiness security process applies to airborne/ground systems within a certification program. The SIM-only program has no airborne system and no certification basis, so it carries no airworthiness-security obligation. The process may be adopted **selectively as engineering practice** for the simulator's trust boundaries. | Threat conditions and security assurance levels are undetermined absent a selected system and basis; the untrusted-interface analysis in [system boundary](system-boundary.md) is engineering input, not a security certification artifact. | Determined at system and basis selection. |
| STD-051 | Airworthiness security methods (DO-356A / ED-203A) | DO-356A / ED-203A | `not applicable` (current scope) | Methods standard supporting DO-326A; inherits STD-050's applicability. | As STD-050. | Determined with STD-050. |
| STD-052 | Continuing airworthiness security (DO-355 / ED-204) | DO-355 / ED-204 | `not applicable` (current scope) | Continuing-airworthiness security applies to fielded certified systems; none exists. | As STD-050. | Determined with STD-050. |

The security family here is the DO-326A / ED-202A airworthiness-security set.
DO-407 / ED-326 is **not** a security standard; it is the synthetic-vision MASPS
and is classified under [SVS / SVGS / CVS vision systems](#svs--svgs--cvs-vision-systems),
STD-066.

### Display and human factors

| ID | Standard / reference | Selected revision | Authority status | Rationale | Known gaps | Issue paper / authority agreement |
| --- | --- | --- | --- | --- | --- | --- |
| STD-060 | FAA AC 25-11B — electronic flight deck displays | AC 25-11B (guidance) | `authority-accepted` (guidance) | Recognized FAA guidance for electronic flight deck displays; used as the human-factors and display-integrity design reference for the PFD/HSI functions ([`AIR-OUT-001`](requirements.md#air-out-001)). | Applied as engineering input; no display is submitted for approval. Compliance is not asserted. | Regulatory paragraph mapping follows aircraft class selection. |
| STD-065 | [FAA AC 20-181](https://www.faa.gov/regulations_policies/advisory_circulars/index.cfm/go/document.information/documentID/1023886) — airworthiness approval of Attitude Heading Reference System (AHRS) equipment | AC 20-181 (issued 2014-04-07; active) | `authority-accepted` (active FAA guidance) | Active FAA advisory circular supplementing airworthiness approval of AHRS articles approved under TSO-C201. This is **source-equipment** guidance, not general display human factors: the instrument functions consume AHRS-class attitude and heading data ([`AIR-IN-001`](requirements.md#air-in-001), [`AIR-IN-005`](requirements.md#air-in-005)), so AC 20-181 governs the eventual attitude/heading source rather than the display itself. | The SIM program supplies no airborne AHRS; applicability attaches to the attitude/heading source of a selected installation, not to browser demonstrations. | None to adopt; applies to the selected AHRS source equipment and its TSO. |

### SVS / SVGS / CVS vision systems

Synthetic-vision content here is **supplemental** situation awareness only
([`AIR-OUT-005`](requirements.md#air-out-005)); no operational vision credit is
sought. These rows carry an explicit **Authority acceptance** column because a
released MASPS and an authority-recognized means of compliance are not the same
thing, and the two diverge most sharply for the vision standards.

| ID | Standard / reference | Selected revision | Authority status | Authority acceptance (2026-07-12) | Rationale | Known gaps | Issue paper / authority agreement |
| --- | --- | --- | --- | --- | --- | --- | --- |
| STD-061 | [FAA AC 20-167B](https://www.faa.gov/regulations_policies/advisory_circulars/index.cfm/go/document.information/documentID/1044323) — airworthiness approval of EVS/EFVS/CVS equipment | AC 20-167B (guidance) | `authority-accepted` (guidance) | Accepted (active FAA AC; AC 20-167A superseded, re-verified 2026-07-13) | Recognized FAA approval guidance for enhanced and combined vision equipment; AC 20-167B supersedes the earlier AC 20-167A. An engineering input while these vision functions are supplemental. | Operational EVS/EFVS credit would need the full guidance set, a performance standard, and a safety case not in current scope. | Required if operational EVS/EFVS credit is ever pursued. |
| STD-062 | [FAA AC 20-185A](https://www.faa.gov/regulations_policies/advisory_circulars/index.cfm/go/document.information/documentID/1039337) — airworthiness approval of SVS/SVGS/ASA-SVS equipment | AC 20-185A (guidance) | `authority-accepted` (guidance) | Accepted (active FAA AC; AC 20-185 superseded, re-verified 2026-07-13) | Synthetic-vision approval guidance; AC 20-185A supersedes the earlier AC 20-185. Same conditional applicability as STD-061 for SVS/SVGS content. | As STD-061, for synthetic-vision content. | Required if operational SVS credit is pursued. |
| STD-063 | RTCA DO-315 / EUROCAE ED-179 — earlier MASPS for EVS/SVS/CVS/EFVS | DO-315 / ED-179 family | `latest engineering baseline` (earlier MASPS) | Not an FAA-named means of compliance; earlier engineering baseline, refined by STD-066 | Earlier vision-systems MASPS, retained as engineering reference; the current harmonized MASPS is DO-407 / ED-326 (STD-066). | No performance credit claimed; the applicable member of the family depends on the SVS function selected. | Required if operational SVS/EFVS performance credit is pursued. |
| STD-066 | [RTCA DO-407](https://www.rtca.org/news/new-rtca-technical-products-address-global-aviation-functions-and-performance/) / [EUROCAE ED-326](https://www.eurocae.net/product/ed-326-masps-for-svs-svgs-cvs/) — MASPS for SVS, SVGS, and CVS | DO-407 (RTCA-approved 2024-12-12) / ED-326 (published 2025-01) | `latest engineering baseline` (released MASPS) | Not accepted — no recognizing FAA AC identified as of 2026-07-12; recognition **to confirm** at authority engagement (STD-066 open action) | Current harmonized RTCA SC-213 / EUROCAE WG-79 MASPS refining SVS, ASA-SVS, SVGS, and CVS performance for head-down and head-up displays; an engineering reference while SVS remains supplemental. Released engineering standard, distinct from the DO-326A / ED-202A security family. | No operational SVS/SVGS/CVS credit is sought; the applicable performance section depends on the function selected. | Required if operational SVS/SVGS/CVS credit is pursued; authority recognition of DO-407 / ED-326 must be confirmed first. |
| STD-064 | SVGS / low-visibility operational credit guidance | Not selected | `not applicable` (current scope) | n/a — no SVGS function present | The baseline supplies **no** Synthetic Vision Guidance System function or low-visibility operational credit ([`AIR-OUT-009`](requirements.md#air-out-009)); the governing SVGS guidance is therefore not applicable. | Adding SVGS requires a distinct intended function, safety assessment, performance standard, and approval basis. | Required only if an SVGS function is introduced. |

### Development-process areas

| ID | Standard / reference | Selected revision | Authority status | Rationale | Known gaps | Issue paper / authority agreement |
| --- | --- | --- | --- | --- | --- | --- |
| STD-070 | Configuration management | DO-178C SCM objectives; ADR-0015 process | `authority-accepted` (framework) | DO-178C software configuration management objectives are the framework. Today the git history, pull-request record, and the ADR-0015 workspace quality gates form the configuration record (see [evidence plan](evidence-plan.md)). | The DO-178C SCM objective set scales by software level and is **deferred** until AIR-02 allocation (issue #27 `PENDING`). Prototype history is a configuration record of engineering work, not certified lifecycle configuration data. | None to adopt the framework; objective applicability follows level allocation. |
| STD-071 | Quality assurance | DO-178C SQA objectives | `authority-accepted` (framework) | DO-178C software quality assurance objectives are the framework; CI quality gates are the current engineering practice. | SQA objectives are **deferred** until level allocation (issue #27 `PENDING`); an independent SQA function is not established. | Follows level allocation. |
| STD-072 | Problem reporting | DO-178C problem-reporting objectives | `authority-accepted` (framework) | Problem reporting is currently the GitHub issue and pull-request record. DO-178C problem-reporting objectives are the framework for a certified build. | Formal problem-report classification and closure discipline are **deferred** until level allocation (issue #27 `PENDING`). | Follows level allocation. |

### Installation and flight test

| ID | Standard / reference | Selected revision | Authority status | Rationale | Known gaps | Issue paper / authority agreement |
| --- | --- | --- | --- | --- | --- | --- |
| STD-080 | Installation approval and flight-test evidence | Not selected | `not applicable` (current scope) | Installation approval, ground test, and flight-test evidence require an installed system on a selected aircraft. The SIM-only program has none; the airborne optical HUD and installation are explicitly outside the boundary ([`AIR-OUT-008`](requirements.md#air-out-008)). | Entirely conditional on aircraft, installation, and certification-basis selection. | Determined at installation; not applicable until an installation exists. |

## Classification decisions worth flagging

- **ARP4754B vs ARP4754A, and ARP4761A vs ARP4761.** The current industry
  revisions (ARP4754B, ARP4761A) are recorded as `latest engineering baseline`
  items, but the **anchor** selected for a basis is the revision that active FAA
  AC 20-174 recognizes (ARP4754A, and the original ARP4761 alongside it). Using
  the newer revisions as a certification basis requires authority agreement. This
  is deliberate: adopting the newest document and implying it is accepted would be
  exactly the misleading move this matrix exists to prevent.
- **DO-160H is not listed as applicable.** DO-160G is the current released
  revision per RTCA and is recorded as the revision to use when equipment is
  selected. DO-160H is in work; it is named here only to state that it is **not**
  an applicable released standard and must not be cited as a basis until it is
  published and the authority disposition it.
- **Aeronautical data.** DO-200C / ED-76B is the current engineering baseline,
  but AC 20-153B recognizes the revision it names; the anchor is therefore the
  recognized revision, with the newer one tracked as `requires authority
  agreement`.
- **Security for a SIM-only program.** The DO-326A / ED-202A family (STD-050..
  STD-052) is classified `not applicable` as a *certification obligation* because
  there is no airborne system or certification basis — not because security is
  unimportant. It may be adopted selectively as engineering practice.
- **FAA vision advisory circulars — active revisions.** The active EVS/EFVS/CVS
  guidance is **AC 20-167B** (STD-061), which supersedes AC 20-167A; the active
  SVS/SVGS/ASA-SVS guidance is **AC 20-185A** (STD-062), which supersedes
  AC 20-185. The superseded revisions are recorded as superseded, never as the
  selected active revision. These identities are held in
  [`standards-registry.toml`](standards-registry.toml) and enforced against this
  matrix by `scripts/check-standards-registry.sh`.
- **DO-407 / ED-326 are synthetic-vision MASPS, not security.** DO-407 (RTCA,
  approved 2024-12-12 by SC-213) and ED-326 (EUROCAE, published 2025-01 by WG-79)
  are the current harmonized Minimum Aviation System Performance Standards for SVS,
  SVGS, and CVS. They are classified under vision systems (STD-066) as a released
  engineering standard. Their **authority acceptance is recorded separately**: no
  recognizing FAA AC was identified as of 2026-07-12, so acceptance is *not
  established* and is tracked as an open verification action.
- **AC 20-181 is AHRS-equipment guidance.** FAA AC 20-181 (issued 2014-04-07,
  active) governs airworthiness approval of AHRS equipment under TSO-C201. It is
  reclassified (STD-065) as active source-equipment guidance for the attitude and
  heading sources the display consumes ([`AIR-IN-001`](requirements.md#air-in-001),
  [`AIR-IN-005`](requirements.md#air-in-005)), not as a general display
  human-factors reference.
- **DO-254, environmental, installation, SVGS.** These are `not applicable` under
  the current SIM-only scope and are retained so their exclusion is explicit and
  revisited when scope changes, rather than silently dropped.

## Open verification actions

Any matrix row whose status is left unverified, to-verify, or to-confirm must
have a corresponding entry here, keyed by its STD identifier, with the concrete
unresolved action. `scripts/check-certification-claims.sh` fails CI if a row is
marked unverified/to-verify/to-confirm/TBD without a matching entry in this
section, so an open question can never quietly satisfy a coverage row.

- **STD-066 (DO-407 / ED-326 authority acceptance).** The document identities and
  released status are verified (RTCA DO-407 approved 2024-12-12; EUROCAE ED-326
  published 2025-01). What is unresolved is **authority acceptance**: no FAA
  advisory circular recognizing DO-407 / ED-326 as a means of compliance was
  identified as of 2026-07-12. Action: at authority engagement, confirm with the
  selected authority whether DO-407 / ED-326 (or a later revision) is recognized
  before it is cited as a certification basis for any SVS/SVGS/CVS credit.

## Re-verification clause

Every status in this matrix is provisional. Before any of it is used to plan a
certification effort, each row must be re-verified against: the current revision
published by the standards body, the advisory material recognized by the
**selected** authority at that time, and the aircraft, operation, installation,
and certification basis chosen for the program. Until then this matrix is an
engineering planning input and confers no compliance credit.
