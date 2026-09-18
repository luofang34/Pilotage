# ADR-0042: Compose an agent as a client module behind a model port

- Status: Proposed
- Date: 2026-09-17

## Context

An operator can give a vehicle an instruction in words. A language model can read
the words. The project needs one rule for how such a model takes part in control.

[ADR-0025](0025-client-optional-operation-automation-principals.md) decides that an
agent is a principal. It holds leased scopes under the same fencing as a person.
[ADR-0037](0037-modular-operator-client-composition.md) decides that an operator
client is a composition of client modules. Each module has a shared core and
platform ports. [ADR-0041](0041-one-mission-sequencing-core.md) decides that one
core sequences missions from a mission document.

`tools/intent-pilot` flew a first agent as a standalone tool. Its measurements set
the limits for this decision. The measured model was a parallel constrained-decoding
engine on a 1.5 B parameter instruction model.

- The model read the destination from one new operator message at 20 of 22 on a
  held-out set. The target of 95 % was not met.
- On a vocabulary of eleven directive kinds with number slots, the same engine read
  19 of 46 held-out instructions. A keyword baseline with no model read 31, the same
  weights with autoregressive decoding read 36, and a 4 B parameter model read 45.
- The model did not select a flight action from vehicle state (8 of 22). Wrong
  answers had a probability of 1.00.
- The model did not find a hazard when the alert text did not contain the label
  word (0 of 4). The answers had a probability above 0.9.
- The model read a message history as one instruction. A recall was found in 0 of 4
  histories and in 5 of 5 single messages.
- The engine fills enumeration fields and boolean fields only. It cannot give a
  number, such as a heading.

A different model will have different limits. The project will compare models, and
some models will read images from one or more cameras.

## Decision

### The agent is a client module

The **agent module** is a client module in the sense of ADR-0037. Its shared core
has no I/O. Its ports connect it to a platform.

The agent module gets its inputs as each module does: typed module inputs from the
client-session core. It reads telemetry, authority events, navigation guidance and
media from the same catalog. It does not open a second connection to the host, and
it has no private data path.

The agent module gives its outputs as a person's control module does. It announces
a profile identity of its own. It requests a lease for a scope by name. It sends
typed control frames and discrete actions under that lease. It offers the lease when
a different principal requests the scope. A client shows the agent as the holder of
the scope.

The module has two platform ports, and the two fly through one type of the shared
core, `AgentFlight`. That type owns the model request, the checks of a model reply,
the executor and the newest vehicle state, so the two ports cannot differ in what
they fly.

- The headless port is a client of its own. It holds a session and a lease.
- The web port is a part of the operator client. The agent is an input source of
  the control runtime that the web shell and the Apple shell share, beside the
  keyboard and a pad. An engage and a release take the transactional neutral
  handover that a device change takes. While the agent is the source, the
  announcement names the identity of the agent as the device profile. The demand of
  the agent goes through the control loop, the publish gate, the typed intent
  builder and the action tracker of the client. The first operator input releases
  the agent, and the press that takes control does not also fire its action.

The web port reads the operational estimate that the instrument module already
admitted, and the advertisement of the motion scope. It has no connection of its
own.

### The model is not part of Pilotage

A model runs as a separate process. Pilotage does not link a model, and Pilotage
does not select a model. The **model port** is the only boundary.

The model port is a request and a reply, one JSON object on each line.

A request has these parts:

- the newest operator message;
- the directive kinds that the agent can fly now, and the permitted value of each slot;
- the names that the message can refer to, with a short legend;
- zero or more **frames**. A frame has a source identity, a source role, a capture
  stamp, a media type, a size, and a projection. The projection is `rectilinear` or
  `equirectangular`. A panorama is one equirectangular frame or a set of rectilinear
  frames with different source identities.

A request does not contain vehicle state or earlier messages. The measurements
above are the reason. An adapter that needs them for one model is a change to the
port, and not a field of the request.

A reply has one directive, a probability for each slot when the model has one, and
the model time. A model can reply `unable`. An adapter must reply `unable` when the
model cannot fill a slot that the directive needs.

A client that cannot start a process, such as a browser, reaches the adapter
through a **model gateway**. The gateway holds the adapter process and gives the
same request and the same reply over HTTP. It adds no second protocol, and it holds
no authority: it sends nothing to a vehicle and reads nothing from one. A page can
call it only when the origin of the page is in its permitted list.

A **model adapter** is the small program that connects one model to the model port.
It owns the prompt, the decoding method and the image encoding of that model. It
declares what it supports: directive kinds, frames, and projections.

### One directive vocabulary

A **directive** is one instruction that the agent can fly. The vocabulary follows
the phrases of air traffic control, because an operator already knows them.

| Directive | Operator phrase, as an example | Slots |
| --- | --- | --- |
| `takeoff` | "cleared for takeoff" | — |
| `direct_to` | "proceed direct BRAVO" | fix, arrival behaviour |
| `heading` | "turn left heading 270" | heading in degrees, turn direction |
| `altitude` | "climb and maintain 30 metres" | height in metres |
| `speed` | "reduce speed to 2" | speed in metres per second |
| `hold` | "hold at ALPHA", "hold present position" | fix or present position |
| `join_procedure` | "cleared RNAV approach runway 27" | procedure name |
| `land` | "cleared to land" | — |
| `go_around` | "go around" | — |
| `return_to_base` | "return to base" | — |
| `unable` | a message that is not an instruction | reason |

The directive vocabulary is not a second mission vocabulary. Each directive lowers
onto the flight actions of ADR-0041, and the function `flight_actions` of the agent
core is the one place that states the lowering. Its match is exhaustive, so a new
directive kind does not compile until it has a lowering. A test gives each lowered
action to the mission core in a document.

| Directive | Flight actions |
| --- | --- |
| `takeoff` | `arm`, `climb` |
| `direct_to` | `follow_plan`, then `land` or `maintain_target` |
| `heading` | `heading` |
| `altitude` | `climb` |
| `speed` | `speed` |
| `hold` | `maintain_target`, after `follow_plan` for a hold at a fix |
| `join_procedure` | `follow_plan` |
| `land` | `land` |
| `go_around` | `go_around` |
| `return_to_base` | `follow_plan`, `land` |

A flight plan comes from the flight-planning module. The lowering asks for the plan
to a fix or along a procedure, and it has no answer when no plan is known.

The executor of the agent core does not run these actions. A directive changes the
active target in flight, and the mission document of ADR-0041 does not change after
a mission starts. That is the addition that the mission core needs before the agent
can fly through it. The agent core has an executor of its own for that reason. That
executor sends the same typed velocity frames.

### Checks before a directive is flown

A reply passes three checks before the executor takes it. The agent core owns the
checks, and none of them uses a model.

1. The **envelope check** refuses a directive kind, a name or a number that is not
   permitted now. It does not clamp a number, because a clamped number is a different
   instruction.
2. The **grounding check** refuses a name or a number that is not in the words of the
   operator message. It also refuses a turn side that the message does not name. The
   envelope check cannot do this: a model gave a permitted fix for a fix that does not
   exist, and the vehicle started to fly to it. A wrong number becomes a refusal and
   not a flight.
3. The **chart check** refuses a name that the executor cannot find.

A refused directive goes to the run record, and the vehicle keeps its last directive.

### Decisions that a model does not make

Deterministic code makes each decision that depends on vehicle state. Examples are
arm, arrival, touchdown and disarm. A hazard response is deterministic code too,
when the telemetry carries the state that it needs (see the open questions). A
model does not make a decision that protects the vehicle.

A verdict never uses the output of a model. The verifier reads the expected result
from the scenario and the position from a truth source. A wrong directive then
fails the run.

### One harness for all models

One harness scores each model adapter with the same case suites. A case has an
operator message, optional frames, and the expected directive with its slots.

The harness applies these rules:

- It records the digest of the suite, the adapter declaration and the model identity
  in each result.
- It keeps an append-only ledger. Each result states how many times that adapter ran
  that suite digest before. A first run of a held-out suite is thus visible, and a
  repeated run is visible too.
- It reports accuracy for each directive kind and each slot, and it reports `unable`
  separately from a wrong answer.
- It reports the probability of wrong answers, so a reader can see if the
  probability separates correct answers from wrong answers.

A closed-loop flight with the verifier is the second level of the harness. It uses
the same scenarios for each model.

## Open questions

- An operator message can hold two instructions, such as "take off and go to ALPHA".
  The model port gives one directive for each message, and a measured model kept the
  first instruction only. A reply with a sequence of directives is one answer. A rule
  that the operator gives one instruction in each message is a second answer.
- The host grant path does not carry the authority class. A client cannot show that
  a holder is an agent. ADR-0025 requires this. The change touches the authority
  engine, the session events and each client.
- A procedure for `join_procedure` comes from navigation data. The agent core takes
  a procedure as a named sequence of fixes. The source of that sequence in a client
  is the flight-planning module.
- The telemetry has no energy state. A hazard response for low energy needs one.
- The permitted pace of an agent, and the classes that can take a scope from an
  agent, are rows of the authority policy matrix of ADR-0010.

## Consequences

- A model can change and Pilotage does not change. A model with images uses the
  same port as a model with text.
- The project can state, with numbers, what each model can do. A weak model stays
  useful for the slots that it fills well.
- The agent cannot do more than a person's client can do. A person can take the
  scope from it in the normal way.
- The agent core has an executor that the mission core will replace. The two must
  not grow apart. The lowering function and its tests hold the two vocabularies
  together. The addition to the mission core is named above.

## Alternatives considered

- **A model inside the host.** Rejected. It would be a privileged control path, which
  ADR-0025 rejects, and it would bind the host to one model runtime.
- **A model that sends control frames.** Rejected. The measured model does not decide
  correctly from vehicle state, and its pace is slower than the frame deadline.
- **One prompt with state, history and images for each model.** Rejected. The measured
  model read state and history as instructions. Each adapter must own its prompt.
- **A free-text reply from the model.** Rejected. A directive with slots can be
  checked against the advertised capability before it is flown.
