# ADR-0041: Use one mission sequencing core

- Status: Proposed
- Date: 2026-08-27

## Context

An operational flight and a calibration trial both use an ordered sequence of
phases. Each phase can have entry conditions, an action, completion conditions,
abort conditions, and a deadline. Each action can need an acknowledgment. A
failure can require cleanup.

The two mission types use different actions. An operational flight uses flight
actions. A calibration trial can use simulator-only actions. The system must
keep these action types separate. It must also use one sequencing
implementation for both mission types.

Pilotage depends on Aviate through a fixed Git revision. Aviate cannot depend
on Pilotage. The design must keep this dependency direction. Aviate also needs
a small runner for its internal continuous-integration missions.

[ADR-0036](0036-situational-domain-ownership.md) defines the identity fields
for an immutable flight-plan handoff. The mission document must use compatible
identity fields. The engine must also bind evidence to its runtime
implementation identity.

## Decision

### Mission document

A `MissionDocument` is the canonical input to mission execution. It contains
an identity, an execution policy, and ordered phases. The document is versioned
and immutable. Its identity contains a revision ID, a schema version, a content
digest, and the applicable navigation-data identity.

The navigation-data identity contains the navigation-data cycle, snapshot ID,
and snapshot digest from ADR-0036. A mission that uses a flight plan includes
an immutable flight-plan reference. The reference contains the plan ID, plan
content digest, and navigation-data identity. It does not contain waypoint
data. A flight handler resolves this reference to a Navigate plan. Units,
datum, and validation evidence stay in the FlightPlanning handoff because the
flight handler checks that handoff when it resolves the reference. The mission
document does not copy these fields.

The mission document contains ordered phases. Each phase contains:

- a stable phase ID;
- its required capabilities;
- its entry conditions;
- one action;
- its completion conditions;
- its abort conditions; and
- its maximum simulator time.

The action vocabulary uses nested enums. `MissionAction::Flight(FlightAction)`
contains operational flight actions. `MissionAction::Trial(TrialAction)`
contains calibration actions. An implementation can recognize an action before
it implements that action. It returns a typed refusal for an action that it
does not implement.

### Sequencing core

Pilotage has one production mission sequencing core. The core is synchronous.
It does not perform input or output. A host supplies time, observations, action
results, and receipts. The core returns typed directives, evidence events,
deadlines, and a terminal result.

The core owns:

- mission admission and capability checks;
- the active phase index and all phase transitions;
- caller-supplied logical time;
- simulator-time and wall-time deadlines;
- action IDs and correlated receipts;
- retries and typed refusals;
- abort behavior and cleanup inside one mission;
- evidence events; and
- the terminal result.

Flight handlers and trial handlers calculate directives for the active phase.
They do not change the active phase. The flight handlers use Navigate for route
geometry, leg sequencing, fusion, and guidance. The trial handlers calculate
calibration directives.

The mission core crate owns the mission document and the sequencing core. It
does not depend on Navigate or an aeronautical-context implementation. The
flight handlers are in a separate crate. This crate depends on the mission core
and on Navigate. The direct dependency allowlist for `flight-tune` stays small.

### Hosts and transport lanes

One engine library has two hosts. The session host runs operational missions.
The campaign runtime runs calibration missions. A calibration mission does not
run through the session host. Lane injection and test-stand control do not use
the session command vocabulary.

The directive vocabulary defines separate typed transport lanes. An
operational command and a simulator-only stimulus have different types. A
real-vehicle host rejects each simulator-only phase during mission admission.
It does not send a simulator-only directive to a real vehicle.

The engine owns cleanup inside one mission. The `Tuner` open transaction owns
the simulator session, vehicle binding, and other resources around mission
runs.

### Component ownership

| Component | Owns | Does not own |
|---|---|---|
| Mission core crate | Mission documents, identity, phases, actions, conditions, validation, content digests, admission, phase transitions, deadlines, receipts, cleanup, evidence events, and terminal results | Navigate algorithms, campaign policy, host I/O, or vehicle actuation |
| Flight handlers crate | Navigate-backed directives for flight phases and flight-plan reference resolution | Phase transitions, campaign policy, or transport I/O |
| Navigate | Route geometry, leg sequencing, fusion, and guidance algorithms | Mission phase transitions, flight-control actuation, or calibration policy |
| `flight-tune` | Candidate search, campaign scheduling, scoring, promotion, and the evidence journal | Mission phase transitions or simulator command execution |
| `pilotage-trial` | Trial samples, evidence contracts, condition artifacts, and manifests | Mission phase transitions or host I/O |
| Aviate | Typed command execution, plant-condition application, telemetry, and receipts | Pilotage product mission sequencing |
| Session host | The operational instance of the mission engine and its operational I/O | Calibration lane injection or test-stand control |
| Campaign runtime | The calibration instance of the mission engine and its calibration I/O | Operational session commands or campaign scoring policy |

### Aviate internal runner

Pilotage keeps the dependency direction from Pilotage to Aviate. Aviate does
not depend on Pilotage.

Aviate keeps a thin internal runner only for its internal
continuous-integration missions. The runner consumes the same typed directive
surface as the Pilotage-facing backend. It does not contain product mission
sequencing. The project removes the current `MissionRunner` only after all
Aviate internal missions use the thin runner.

### Runtime identity and migration order

A change to the mission engine resets the runtime implementation identity.
Evidence from a different runtime implementation identity is orphan evidence.
The project does not use orphan evidence for a new qualification decision.

The migration uses this order:

1. Define the mission document schema with the `Flight` and `Trial` action
   vocabularies.
2. Make each action type recognizable. Return a typed refusal for each action
   that has no implementation.
3. Adopt the sequencing core in the session host and the campaign runtime.
4. Reset the runtime implementation identity.
5. Include production-source renames in the same identity-reset window. Rename
   the `flight-tune` `engine.rs` module to `campaign.rs`. Rename the
   `flight-tune` `SimulatorBackend` trait in this window.
6. Repeat the bench campaign with the new runtime identity.
7. Complete the adoption and identity reset before the first live calibration
   campaign.

## Open questions

The open question is whether the trial runtime implementation identity contains
only the sequencing core and the trial handlers, or also contains the flight
handlers. This record does not answer the question. The project defers the
answer to a separate architecture decision. The project must record that
decision before the first live calibration campaign.

## Consequences

- Operational missions and calibration missions use one phase machine.
- The two hosts use the same engine library and keep different I/O paths.
- Nested action enums keep operational and simulator-only directives separate.
- The mission core crate stays independent of Navigate and aeronautical-context
  implementations.
- Navigate remains the owner of route and guidance algorithms.
- A real-vehicle host cannot admit a simulator-only phase.
- `flight-tune` keeps campaign policy and does not get a second mission engine.
- Aviate keeps its repository independence and a small internal test runner.
- Engine changes can require new evidence because they reset runtime identity.

## Alternatives considered

- **Add a `ScenarioEngine` to `flight-tune`:** Rejected. It creates a second
  phase machine and duplicates deadlines, receipts, cleanup, and terminal
  behavior.
- **Put all actions in one large enum and one large file:** Rejected. Nested
  action types keep the two directive vocabularies separate.
- **Make the core depend on Navigate:** Rejected. Calibration does not need a
  route or navigation-data implementation.
- **Run calibration through the session host:** Rejected. Simulator lane
  injection and test-stand control do not belong in the operational session
  vocabulary.
- **Make Aviate depend on Pilotage:** Rejected. The repository dependency points
  from Pilotage to Aviate.
- **Remove the Aviate runner before its mission migration:** Rejected. Aviate
  needs an internal runner for its continuous-integration missions.

SIM / NOT FOR FLIGHT.
