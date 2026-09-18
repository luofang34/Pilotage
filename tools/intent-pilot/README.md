# intent-pilot

`intent-pilot` is a Pilotage control source. A language model reads each
operator message. A deterministic sequencer flies the vehicle. A verifier
gives the verdict from simulator truth.

## Terms

- **Operator message**: one line of free text from the operator.
- **Intent**: a destination waypoint and an arrival behaviour (`LAND` or `HOLD`).
- **Classifier**: the process that reads an operator message and gives an intent.
- **Sequencer**: the state machine that changes an intent and the vehicle state
  into motion demands and arm or disarm requests.
- **Verifier**: the module that compares simulator truth with the scenario.
- **Scenario**: a JSON document with waypoints, operator messages, and the
  expected result.
- **Run record**: a JSON Lines file with one entry for each event of a flight.

## How the control source connects

The tool is a WebTransport client of `pilotage-session-host`. It uses
`pilotage-client-session::ClientEngine`, as the native clients do. The host
needs no change.

1. The tool pins the SHA-256 of the host certificate. The tool has no mode
   without a pinned certificate.
2. The tool announces the profile identity `automation.intent-pilot/v1`. The
   host records that identity as the source of the control frames.
3. The tool requests the lease for the scope `vehicle.motion`. It selects the
   scope by name.
4. The tool sends one typed `Velocity` frame each 50 ms. The host drops a
   holder that is silent for 1 s.
5. The tool sends `Arm` and `Disarm` on the reliable action stream. It sends
   `Disarm` only when the scope advertises `Disarm`.
6. When a different principal requests the scope, the tool offers the
   transfer. Then the tool stops. It does not request the lease again.

## How content gets to the classifier

The classifier is a child process. The tool writes one JSON line and reads one
JSON line.

Request: `{"message": "...", "targets": ["ALPHA", "BRAVO", "HOME"], "legend": "..."}`

Reply: `{"target": "BRAVO", "on_arrival": "LAND", "target_prob": 0.96, "on_arrival_prob": 0.95, "model_ms": 176.0}`

The request contains only the newest operator message, the permitted target
names, and the waypoint legend. The request does not contain vehicle state.
The request does not contain earlier messages. The measurements in
"Measured limits" give the reason.

The tool calls the classifier only when an operator message arrives. The frame
loop does not wait for the reply.

`classifier/rlcd_intent_service.py` connects the parallel constrained-decoding
engine from the Hugging Face repository `harshatheg/Qwen-2.5-1B-RLCD`. That
repository contains no model weights. The engine loads
`mlx-community/Qwen2.5-1.5B-Instruct-4bit`. A different classifier can replace
it if it uses the same JSON lines.

## Measured limits of the classifier

These results are from probes on an Apple M4 with 16 GB. Each held-out set was
written before its first run and was run one time.

| Task | Result | Decision |
| --- | --- | --- |
| Select the next flight action from vehicle state | 1 of 9, then 8 of 22. Wrong answers had a probability of 1.00. | The sequencer makes these decisions. |
| Classify a hazard when the alert text does not contain the label word | 0 of 4. The answer was `NONE` with a probability above 0.9. | Hazard detection is not given to the classifier. |
| Find a recall to `HOME` in a message history | 0 of 4 | The classifier gets the newest message only. |
| Find the destination in one new message | 20 of 22. The target of 95 % was not met. | The verifier does not use the classifier output. |
| Find the destination in the 6 scenario messages | 6 of 6 | — |

The probability from the classifier is not a safe gate. Wrong answers had
probabilities up to 0.99.

## Verdict rules

The verifier reads the expected result from the scenario. It reads the
position from simulator truth. It reads the armed state from the flight
controller report. It does not read the classifier output or the sequencer
phase. Thus an incorrect intent causes a `FAIL` verdict.

- `PASS`: truth shows each `must_approach` checkpoint in sequence. Then truth
  shows the end state for 3 s. For `LAND`, the vehicle is in the arrival
  radius, on the ground, not armed, and not moving. For `HOLD`, the vehicle is
  in the arrival radius and at cruise height.
- `FAIL`: truth enters the arrival radius of a `must_not_reach` waypoint, or
  the time limit ends.
- `INCONCLUSIVE`: no simulator truth arrived. This is not a `PASS`.

A vehicle that reports height must climb above 1 m before an end state at
`HOME` is valid.

Exit status: `0` pass, `1` fail, `2` inconclusive, `3` error.

## Run

```
intent-pilot --url https://127.0.0.1:4433/pilotage \
  --cert <sha256-hex from the host LISTENING line> \
  --scenario tools/intent-pilot/scenarios/retarget-recall.json \
  --record run.jsonl \
  --classifier-cmd "PYTHONPATH=<engine repository> python tools/intent-pilot/classifier/rlcd_intent_service.py"
```

Add `--planar-truth` only for the host reference adapter. That adapter has a
planar vehicle, and its pose is the simulator state.

## Flight results

Vehicle: Aviate x500 in Gazebo Harmonic, physics only. Host: `pilotage-session-host`
at `b57c7c2`. Date: 2026-09-17. The verdict of each flight is from stamped simulator
truth.

| Scenario | Flights | Pass | End distance from the expected waypoint |
| --- | --- | --- | --- |
| `direct-land` | 3 | 3 | 0.50 m, 0.51 m, 0.58 m |
| `retarget-land` | 3 | 3 | 0.33 m, 0.32 m, 0.43 m |
| `retarget-recall` | 4 | 4 | 0.41 m, 0.46 m, 0.38 m, 0.35 m |

The host rejected no control frame. The flight controller reported a disarm after
each landing. Four more attempts did not fly. The cause was the test scripts: they
used more flight-controller restarts than the launcher permits in one session. The
cause was not the pilot.

Before the Gazebo flights, the verifier failed two flights on the host reference
vehicle. The first guidance law had no damping, and the vehicle went 8 m through the
waypoint. The en-route time did not start again when the destination changed. The
two defects are corrected, and each has a test.

In `retarget-recall` the vehicle came 2.5 m to 3.4 m from `BRAVO` before the recall.
The limit for `must_not_reach` is 1.5 m. The margin is smaller than the scenario
author planned.

## Known limits

- The guidance law is a speed schedule with a velocity correction. It is not
  tuned for one vehicle.
- The protocol has no typed `Land` action. The sequencer descends and then
  sends `Disarm`. Aviate refuses `Disarm` while it reports the vehicle
  airborne, so the sequencer sends `Disarm` again each 2 s.
- The telemetry has no battery field. The tool has no hazard response.
- The host grant path does not mark an automation principal as different from
  a person.
