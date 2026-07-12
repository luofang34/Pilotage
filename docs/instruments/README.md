# Instrument intended-function baseline

The files in this directory define what Pilotage instrument displays may do,
where their responsibility ends, and which assumptions must be made explicit.
They are engineering inputs for later safety assessment and implementation.
They are not an FAA/EASA finding, TSO authorization, TC/STC approval, or a claim
that any Pilotage display is suitable for airborne use.

The baseline is split into four controlled artifacts:

- [Intended functions](intended-functions.md) defines each display function,
  operating mode, failure presentation, and source assumption.
- [System boundary](system-boundary.md) identifies trusted and untrusted
  interfaces, simulator-only components, and deterministic reversion paths.
- [Requirements](requirements.md) is the registry of stable instrument
  requirement identifiers.
- [Review record](review-record.md) records the approvals required before the
  baseline can be treated as reviewed.

## Preliminary safety assessment

Seeded from the intended-function baseline, the preliminary functional hazard
assessment and system safety assessment analyse the display functions for loss,
misleading, frozen, stale, wrong-reference, and failed-reversion conditions,
derive safety requirements, and allocate assurance by function and architecture.
They are preliminary and not closable: classifications stay conditional on a
selected vehicle, operation, installation, and certification basis, and closure
requires a qualified independent safety review.

- [Functional hazard assessment](fha.md) is the failure-condition hazard log with
  conditional severities and derived requirements.
- [System safety assessment](pssa.md) is the common-cause and independence
  analysis, assurance allocation, traceability, and AIR-02 review record.

All browser, WebAssembly, Canvas, WebTransport, Gazebo, and test-harness output
is **SIM / NOT FOR FLIGHT**. A visual resemblance to an aircraft display does
not make the output primary flight information or authorize operational credit.

## Standards and lifecycle evidence planning

Seeded from the intended-function and safety baselines, the standards-applicability
matrix and lifecycle-evidence plan record which industry standards and authority
guidance a future certification effort would draw on, and which repository
artifacts are engineering input versus lifecycle evidence still to be established.
Both are **SIM / NOT FOR FLIGHT** engineering planning inputs. They make no
compliance finding and confer no certification credit, and they depend on the
AIR-01 (#24) and AIR-02 (#27) baselines, which remain preliminary and pending
independent review.

- [Standards applicability matrix](standards-applicability.md) classifies each
  standard as authority-accepted, latest engineering baseline, requiring authority
  agreement, or not applicable, with rationale, gaps, and issue-paper needs.
- [Lifecycle evidence plan](evidence-plan.md) indexes today's requirement, design,
  code, test, review, and configuration baselines, and states which prototype
  artifacts may be reused as engineering input versus which DO-178C lifecycle
  evidence must be established anew. Structural-coverage objectives are deferred
  until AIR-02 allocates assurance.

`scripts/check-certification-claims.sh` fails CI on any artifact that asserts a
certification, compliance, approval, or airworthiness claim about this project;
`scripts/trace-report.sh` emits a requirement/test/review/configuration inventory
to the CI log (an inventory, not a compliance trace).

## Change control

Every new or changed display feature must cite at least one requirement from
[the registry](requirements.md) in its issue and pull request
([`AIR-BAS-005`](requirements.md#air-bas-005)). If no applicable
requirement exists, the intended-function baseline is changed and reviewed
before the feature is accepted. Requirement identifiers are permanent: changed
meaning receives a new identifier, while retired requirements remain in the
registry with their disposition.

`scripts/check-instrument-requirements.sh` rejects duplicate identifiers,
malformed identifiers, undefined references, unreferenced requirements, and
mismatched requirement links.

## Design inputs

The conservative architecture reference is a dual-pilot Part 25 IFR flight deck
that may present primary flight information. The target aircraft, operating
rules, certification authority, certification basis, and installed equipment
have not been selected. Consequently, this baseline makes no blanket Design
Assurance Level allocation.

Forward-looking industry material is an engineering input only until the
selected certification authority identifies an accepted revision:

- [FAA AC 25-11B, Electronic Flight Deck Displays](https://www.faa.gov/documentlibrary/media/advisory_circular/ac_25-11b.pdf)
- [FAA AC 25.1309-1B, System Design and Analysis](https://www.faa.gov/regulations_policies/advisory_circulars/index.cfm/go/document.information/documentID/1043037)
- [FAA AC 20-185A, Synthetic Vision Guidance Systems](https://www.faa.gov/documentLibrary/media/Advisory_Circular/AC_20-185A.pdf)
- [SAE ARP4754B, Development of Civil Aircraft and Systems](https://saemobilus.sae.org/standards/arp4754b-guidelines-development-civil-aircraft-systems)
