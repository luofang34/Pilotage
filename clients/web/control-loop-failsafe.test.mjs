// Failure-surface checks for the control loop, driven through the REAL
// createControlLoop factory:
//
//   1. A dead datagram channel (writer.ready rejects) must not exit the
//      loop silently — the host keeps enacting the last sent frame, so the
//      loop must latch, release authority, and surface the loss.
//   2. A key RELEASE must reach the runtime even when the key is no longer
//      bound (bindings re-resolve mid-hold); a swallowed keyup strands a
//      phantom deflection nothing can clear.
//   3. Frame-rejection logging is rate-limited, not once-ever: an ongoing
//      rejection stream is the only visible evidence commands are not
//      enacting.
//
// Run: node clients/web/control-loop-failsafe.test.mjs

import assert from "node:assert/strict";

import { FRAME_REJECTION_RELOG_MS, createControlLoop } from "./control-loop.js";
import { CONTROL_ACTION, MODE_TARGET } from "./wire.js";

globalThis.navigator ??= {};

function harness({ mayPublish = true, agentSource = null } = {}) {
  const record = {
    active: true,
    latched: 0,
    released: 0,
    logs: [],
    keyEvents: [],
    prevented: 0,
    bound: true,
  };
  const state = {
    connected: true,
    selectedPadId: null,
    pendingReset: false,
    actionTracker: { pending: new Map(), nextId: 1 },
    controlCompletion: null,
    stopControlRun: null,
    motionScope: "vehicle.motion",
    resumePendingToken: null,
    resumeGimbalLease: false,
    sessionWriter: null,
    lastFrameRejectionLogged: null,
    controlShell: {
      beginControlRun() {},
      boundKey: () => record.bound,
      keyEvent: (key, pressed) => record.keyEvents.push({ key, pressed }),
      tickFromKeys: () => ({ motion: null, gimbal: null, lease: null, motionLease: null }),
      planAuthority: () => null,
      authority: () => ({ generation: 0n, granted: false, denied: false }),
      authorityEvent: () => "ignored",
    },
  };
  const loop = createControlLoop({
    state,
    els: { flightMode: { value: "rover" }, resumeBtn: { hidden: true, disabled: false } },
    transportSessions: {
      isActive: () => record.active,
      trackWriter: () => true,
      untrackWriter() {},
      currentToken: () => 1,
      runIfActive: (_token, fn) => fn(),
    },
    controlGate: {
      isLatched: () => false,
      mayPublish: () => mayPublish,
      reset() {},
      latchInputLoss: () => {
        record.latched += 1;
      },
    },
    releaseTracker: { isPending: () => false },
    vehicleId: 1n,
    motionScope: "vehicle.motion",
    directScope: "vehicle.motion.direct",
    gimbalScope: "vehicle.gimbal",
    lifecycleScope: "vehicle.lifecycle",
    frameRejectionUplinkIdle: 18,
    controlHz: 200,
    log: (line) => record.logs.push(line),
    surface: new Proxy(
      {},
      {
        get: (_target, name) =>
          name === "controlReleased" ? () => (record.released += 1) : () => {},
      },
    ),
    updateControlReadout() {},
    reportSuppressedPresses() {},
    currentTelemetryHeading: () => null,
    lengthDelimit: (bytes) => bytes,
    maybeAnnounceProfileActivation() {},
    requestReconnect() {},
    agentSource,
  });
  return { loop, state, record };
}

// ---- 1. dead datagram channel: loud authority release ----------------------
{
  const { loop, state, record } = harness();
  const writer = {
    ready: Promise.reject(new Error("datagram stream errored")),
    releaseLock() {},
  };
  writer.ready.catch(() => {});
  const started = loop.startControlLoop(
    { datagrams: { createWritable: () => ({ getWriter: () => writer }) } },
    1,
  );
  assert.equal(started, true, "the loop starts");
  await state.controlCompletion;
  assert.equal(record.latched, 1, "channel death latches the gate");
  assert.equal(record.released, 1, "channel death surfaces control release");
  assert.ok(
    record.logs.some((line) => /datagram channel failed/.test(line)),
    `channel death is logged loudly (got ${JSON.stringify(record.logs)})`,
  );
  record.active = false; // lets the suspended-press watch exit
}

// ---- 2. releases always reach the runtime ----------------------------------
{
  const { loop, record } = harness();
  const event = (key) => ({ key, preventDefault: () => (record.prevented += 1) });

  record.bound = false;
  loop.forwardKey(event("d"), true);
  assert.deepEqual(record.keyEvents, [], "an unbound PRESS is not forwarded");
  loop.forwardKey(event("d"), false);
  assert.deepEqual(
    record.keyEvents,
    [{ key: "d", pressed: false }],
    "an unbound RELEASE still reaches the runtime",
  );
  assert.equal(record.prevented, 0, "unbound keys keep their default behavior");

  record.bound = true;
  loop.forwardKey(event("d"), true);
  loop.forwardKey(event("d"), false);
  assert.deepEqual(record.keyEvents.slice(1), [
    { key: "d", pressed: true },
    { key: "d", pressed: false },
  ]);
  assert.equal(record.prevented, 2, "bound keys are consumed");
}

// ---- 3. rejection logging is rate-limited, not once-ever -------------------
{
  const { loop, state, record } = harness();
  const rejection = { reason: 15, scope: "vehicle.motion", sequence: 7, currentGeneration: 3n };
  loop.handleFrameRejected(rejection);
  loop.handleFrameRejected(rejection);
  loop.handleFrameRejected(rejection);
  assert.equal(
    record.logs.filter((line) => /frame rejected/.test(line)).length,
    1,
    "an immediate repeat is suppressed",
  );
  // Age the last log past the re-log interval: the stream must speak again,
  // carrying the suppressed count.
  state.lastFrameRejectionLogged.atMs -= FRAME_REJECTION_RELOG_MS + 1;
  loop.handleFrameRejected(rejection);
  const relogs = record.logs.filter((line) => /frame rejected/.test(line));
  assert.equal(relogs.length, 2, "an ongoing rejection stream re-logs");
  assert.ok(/\+2 suppressed/.test(relogs[1]), `the re-log counts what it swallowed (${relogs[1]})`);

  loop.handleFrameRejected({ ...rejection, reason: 2 });
  assert.equal(
    record.logs.filter((line) => /frame rejected/.test(line)).length,
    3,
    "a different rejection key logs immediately",
  );
}

// ---- 4. an engaged agent is released before its law or its authority changes
{
  const releases = [];
  const agentSource = {
    release: (reason) => releases.push(reason),
    sync() {},
    tick: () => null,
    overridden() {},
    onActionResult() {},
  };
  const { loop, state } = harness({ agentSource });
  state.controlShell.agentEngaged = true;
  state.controlShell.activationRevision = () => 1;
  state.controlShell.profileRevision = () => 1;
  state.lifecycle = { pendingPress: false };
  state.fpvActive = false;
  state.advertisedScopes = [
    {
      vehicleId: 1n,
      scope: "vehicle.lifecycle",
      intents: [],
      actions: [{ action: CONTROL_ACTION.simReset, modeTargets: [] }],
    },
    {
      vehicleId: 1n,
      scope: "vehicle.motion",
      intents: [],
      actions: [
        { action: CONTROL_ACTION.modeRequest, modeTargets: [MODE_TARGET.fpvDirect] },
        { action: CONTROL_ACTION.feelModeRequest, modeTargets: [], feelTargets: [2] },
      ],
    },
  ];
  state.sessionWriter = { write: async () => {} };
  loop.requestFlightModeSwitch();
  loop.requestSimReset();
  loop.requestFeelMode(2);
  loop.suspendControlForInputLoss(1);
  assert.deepEqual(
    releases,
    [
      "the flight mode changes",
      "the simulation resets",
      "the control-feel law changes",
      "control authority was released",
    ],
    "each change of the law or the authority releases the agent first",
  );
  state.controlShell.agentEngaged = false;
  loop.requestSimReset();
  assert.equal(releases.length, 4, "no release when no agent is engaged");
}

console.log("all control-loop failsafe checks passed");
process.exit(0);
