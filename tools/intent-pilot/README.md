# intent-pilot and agent-eval

This crate is the headless port of the agent client module
([ADR-0042](../../docs/adr/0042-agent-client-module-and-model-port.md)). The
shared core is `crates/pilotage-agent`. The core has no I/O.

- `intent-pilot` flies operator messages. A model reads each message and gives a
  directive. Deterministic code checks the directive and flies it.
- `agent-eval` gives each model the same cases and the same score.
- `model-gateway` gives the model port over HTTP to a client that cannot start a
  process. The browser viewer uses it.

The agent module has a second port in the browser viewer (`clients/web`, with the
wasm crate `clients/web-agent`). The two ports fly through `AgentFlight` of the
shared core, so the checks and the executor are the same.

## Terms

- **Operator message**: one line of free text from the operator.
- **Directive**: one instruction that the agent can fly, such as `direct_to`,
  `heading`, `altitude`, `hold`, `join_procedure`, `land`, `go_around` or
  `return_to_base`. `unable` means that the message is not an instruction.
- **Slot**: a value in a directive, such as a fix name or a heading.
- **Model adapter**: a separate program that connects one model to the model port.
- **Model port**: the request and the reply between Pilotage and a model adapter.
- **Flight envelope**: the directive kinds, names and number ranges that are
  permitted now.
- **Executor**: the state machine that changes a directive and the vehicle state
  into motion demands and arm or disarm requests.
- **Verifier**: the module that compares simulator truth with the scenario.
- **Scenario**: fixes, procedures, scripted operator messages, and the expected result.
- **Suite**: cases for `agent-eval`. A case is an operator message and the expected directive.

## How the agent connects to the host

The agent is a client, as the web client and the tablet client are. The host needs
no change.

1. It pins the SHA-256 of the host certificate. It has no mode without a pin.
2. It announces the profile identity `automation.intent-pilot/v1`.
3. It requests the lease for the scope `vehicle.motion`, by name.
4. It sends one typed `Velocity` frame each 50 ms.
5. It sends `Arm` and `Disarm` on the reliable action stream. It sends `Disarm`
   only when the scope advertises `Disarm`.
6. When a different principal requests the motion scope, it offers the transfer and
   stops. It does not request the lease again.
7. It reads telemetry, authority events and action results from the client-session
   core, as each client module does.

## The model port

A model adapter is a child process. It writes its declaration as its first line.
After that it reads one request line and writes one reply line. Each line is one
JSON object.

```
declaration  {"ready":true,"adapter":"ollama-json/1","model":"gemma4:e4b",
              "kinds":["takeoff","direct_to",...],"frames":{"max_frames":0,"projections":[]}}
request      {"message":"Turn left heading 270.","envelope":{...},"legend":"FIXES: ...","frames":[]}
reply        {"directive":{"kind":"heading","degrees":270,"turn":"left"},
              "probabilities":{"kind":0.98},"model_ms":805.0}
fault        {"error":"..."}
```

A request contains the newest operator message, the flight envelope, a legend, and
zero or more frames. It contains no vehicle state and no earlier messages. A small
model reads state and history as instructions. "Measured results" gives the numbers.

A frame is one image for a model that reads images. It has a source identity, a
source role, a capture stamp, a media type, a size, a projection and the image
bytes. The projection is `rectilinear` or `equirectangular`. A panorama is one
equirectangular frame, or a set of rectilinear frames with different source
identities. No adapter in this directory reads frames. The port and the harness
carry them.

## Three checks before a directive is flown

1. **Envelope.** The directive kind, each name and each number must be permitted. A
   number outside its range is refused. It is not clamped.
2. **Grounding.** Each name and each number of the directive must be in the words of
   the operator message. "two seven zero" is 270, and "RNAV two seven" names
   `RNAV27`. A turn side must be in the message, and a side in the message must be in
   the directive. A model once gave `ALPHA` for a fix that does not exist. `ALPHA`
   is permitted, so only this check stopped it.
3. **Chart.** The executor finds each fix and each procedure in the scenario.

A refused directive is written to the run record. The vehicle keeps its last directive.

## Decisions that a model does not make

The executor decides when to arm, when a climb is complete, when the vehicle is at a
fix, when it is on the ground, and when it is too far away. A heading has no end, so
the executor holds position at `max_range_m`. A climb that does not leave the ground
in 3 s sends a new arm request, because an armed report can be old. When a run ends
in the air, the pilot brings the vehicle home and lands it.

The verifier reads the expected result from the scenario and the position from
simulator truth. It does not read a model reply or the executor phase. Each scripted
message must have an effect that truth can show. A scenario without such a
checkpoint once passed a flight that did not fly its first leg.

## Run a scenario

```
intent-pilot --url https://127.0.0.1:4433/pilotage \
  --cert <sha256-hex from the host LISTENING line> \
  --scenario tools/intent-pilot/scenarios/go-around.json \
  --record run.jsonl \
  --model-cmd "PYTHONPATH=tools/intent-pilot/adapters python3 tools/intent-pilot/adapters/ollama_json.py"
```

Exit status: `0` pass or live run ended, `1` fail, `2` inconclusive, `3` error.
Add `--planar-truth` only for the host reference adapter.

Scenarios: `direct-land`, `retarget-recall`, `vectors` (heading and altitude),
`approach` (a procedure), `go-around`, and `live` (no script).

## Run a live demonstration

1. Start the session with `cargo xtask sim --fc aviate --lan --no-open`. The launcher
   prints a viewer address for a browser on the same network. The address uses the
   name `pilotage.local`. If that name does not resolve, use the host name of the
   machine. The viewer page, the connect manifest and the QUIC port were reached from
   a second machine. The viewer was not operated in a browser in this work.
2. Start the pilot with `--scenario tools/intent-pilot/scenarios/live.json --live`.
3. Type one operator message on each line. Examples: `Cleared for takeoff.`,
   `Turn left heading 270.`, `Climb and maintain 12 metres.`, `Proceed direct BRAVO
   and hold.`, `Cleared RNAV 27 approach.`, `Go around.`, `Return to base.`
4. Type `quit`. The vehicle returns to the launch point and lands. Then the pilot stops.

A live run has no verdict. A person in the viewer can request the motion scope at
any time, and the pilot gives it up.

## Fly from the browser viewer

In the viewer the agent is an input source of the client, beside the keyboard and a
pad. It uses the lease that the viewer holds. It does not open a connection.

1. Start the session with `cargo xtask sim --fc aviate --no-open --viewer-port 8099`.
2. Start the gateway:

   ```
   model-gateway --model-cmd "PYTHONPATH=tools/intent-pilot/adapters python3 tools/intent-pilot/adapters/ollama_json.py"
   ```

   The gateway listens on `127.0.0.1:8098`. It accepts a page from
   `http://localhost:8099` or `http://127.0.0.1:8099`. Give `--listen` and
   `--allow-origin` for a different address.
3. Open the viewer address that the launcher prints. Select a Quad control mode.
4. Push **Engage agent**. The status shows `AGENT HAS CONTROL` and the executor
   phase. The host log shows a profile activation with the device profile
   `automation.intent-pilot/v1`.
5. Type an instruction, or push a preset. The panel shows each directive that is
   flown and each reply that is refused.
6. Move a stick or push a flight key to take control. The agent stops at once, and
   the vehicle holds its position until you move a control again. Push **Engage
   agent** to give control back.

The viewer reads the gateway address from the `agent` parameter of its address,
for example `&agent=http://localhost:8098`. The default is port 8098 on the host
that serves the page.

A browser on a second machine needs a secure context for WebTransport. Forward the
viewer port and the gateway port to `localhost` on that machine, and give the host
name of the session machine in the `host` parameter.

## Score a model

```
agent-eval --suite tools/intent-pilot/suites/atc-heldout-01.json \
  --model-cmd "<adapter command>" --out out/ --ledger ledger.jsonl
```

The result names the suite by its SHA-256. The ledger is append-only. It counts the
runs of each adapter and model on each suite digest, so a number from a second run
cannot pass as a number from a first run. Develop an adapter on `atc-open-01`. Run
`atc-heldout-01` one time, and do not tune on it.

The report gives the score for each directive kind, category and slot. It keeps
`unable`, a refused reply and an adapter fault apart from a wrong answer. It counts
the wrong answers that pass every check, because the agent would fly them.

Adapters in `adapters/`:

| File | Model |
| --- | --- |
| `ollama_json.py` | A model on an Ollama server. JSON reply. |
| `qwen_json.py` | An MLX model. Autoregressive JSON reply. |
| `rlcd_directive.py` | The parallel constrained-decoding engine of `harshatheg/Qwen-2.5-1B-RLCD`. It fills enumeration fields only, so each number is a set of digit fields. |
| `keyword_baseline.py` | No model. Regular expressions from the open suite. |
| `oracle.py` | No model. It answers with the directive that the scenario author wrote. It separates the flight from the quality of a model. |

## Measured results

Machine: Apple M4, 16 GB. Date: 2026-09-17. Suite `atc-heldout-01`, 46 cases, first
run of each adapter. The same person wrote the suite and the adapters, so these
numbers are not a blind test. The comparison between adapters is fair.

| Adapter | Correct | Wrong and flown | Wrong and stopped | Median time |
| --- | --- | --- | --- | --- |
| `ollama_json.py`, `gemma4:e4b` | 45 of 46 | 1 | 0 | 756 ms |
| `qwen_json.py`, `Qwen2.5-1.5B-Instruct-4bit` | 36 of 46 | 7 | 3 | 601 ms |
| `keyword_baseline.py` | 31 of 46 | 3 | 0 | 0 ms |
| `rlcd_directive.py`, the same Qwen weights | 19 of 46 | 5 | 19 | 379 ms |

The "flown" and "stopped" columns are from the second run, before the turn-side rule.
That rule was written after the held-out run showed a reply with a lost turn side. It
changes what the agent flies. It does not change the score of a model.
The grounding check stopped no correct answer. The parallel engine is below the
keyword baseline on this vocabulary: directive kind 21 of 30 and numbers 2 of 15 in
the two suites. It is correct for kinds with one clear word, such as a procedure.

The suite has no message with two instructions. `gemma4:e4b` read "Get airborne and
go to ALPHA, hold overhead." as `takeoff` only. The model port gives one directive
for each message.

Flights: Aviate x500 in Gazebo Harmonic, physics only, `gemma4:e4b`. Each verdict is
from stamped simulator truth.

| Scenario | Flights | Pass | Notes |
| --- | --- | --- | --- |
| `vectors` | 5 | 5 | heading 270 held, 12 m held, landed at `HOME` |
| `approach` | 3 | 3 | `DELTA`, then `BRAVO`, then landed at `HOME` |
| `go-around` | 3 | 3 | descent seen, then 5 m held, landed at `HOME` |
| `direct-land` | 2 | 2 | 0.60 m and 0.71 m from `ALPHA` |
| `retarget-recall` | 3 | 0 | The model misread the first message. One of these flights passed an earlier, weaker expectation. That pass does not count. With `oracle.py` the scenario passes. |

The host rejected no control frame in these flights.

Two live runs took operator messages from the keyboard. In the two runs the message
"Proceed direct ZULU and land." got the reply `ALPHA`. The first run had no grounding
check, and the vehicle flew toward `ALPHA` for 4 s. The second run refused the reply.

### Flights from the browser viewer

A headless Chrome on a second machine flew this sequence through the agent panel,
with `gemma4:e4b` behind the gateway: takeoff, "Turn left heading 270.", "Proceed
direct ZULU and land.", "Proceed direct BRAVO and hold.", a flight key from the
operator, a second engage, and "Cleared RNAV 27 approach.". Each check reads the
vehicle pose from Gazebo (`gz model -p`). It does not read the agent or the page.

| Run | Result | Notes |
| --- | --- | --- |
| 1 | 10 of 11 checks | The failed check was the first probe of the test rig. It read the wrong page element for the lease. Each flight check passed. |
| 2 | lost | The test rig stopped on a failed truth sample. The browser closed, and the link-loss policy of the host landed the vehicle. |
| 3 | 11 of 11 checks | 5.25 m after takeoff, heading 264 and 10.3 m west after 7 s, 0.9 m from `BRAVO`, 5.1 m under the operator, landed 0.19 m from `HOME`. |
| 4 | 11 of 11 checks | Heading 271 and 13.9 m west after 7 s, 0.7 m from `BRAVO`, landed 0.45 m from `HOME`. |
| 5 | 11 of 11 checks | Heading 267 and 13.5 m west after 7 s, 0.4 m from `BRAVO`, landed 0.30 m from `HOME`. This run is the recorded demonstration. |
| 6 | 11 of 11 checks | Flown after the fix of the release source. Heading 265 and 10.3 m west after 7 s, 0.9 m from `BRAVO`, landed 0.28 m from `HOME`. |

In each of the six runs the model gave `ALPHA` for `ZULU`, and the grounding check
refused the reply. The host accepted four profile activations in run 1: the keyboard, the agent,
the keyboard after the operator input, and the agent again. The host rejected no
control frame.

## Known limits

- The executor is a copy of what the mission core of ADR-0041 will do. Each directive
  has a lowering onto the flight actions of the mission core (`flight_actions`), but
  the mission document does not change after a mission starts, so the agent does not
  fly through the mission core.
- The protocol has no typed `Land` action. The executor descends and then sends
  `Disarm`. Aviate refuses `Disarm` while it reports the vehicle airborne, so the
  executor sends `Disarm` again each 2 s.
- The telemetry has no energy state. The agent has no hazard response.
- The host grant path does not mark an agent as different from a person.
- A heading is a heading and not a track. The executor does not hold a ground track.
- The browser port flies only while its window has the focus, as a person's input
  does. A window that loses the focus releases its lease.
- The Apple client links the same control runtime, so it has the agent input source.
  It has no agent panel and no connection to a model gateway.
