// The agent client module in the browser shell (ADR-0042), against the REAL
// agent runtime and the REAL control runtime. A stub stands in for the model
// gateway only: what a model says is the input of each case.
//
// Build the wasm first: scripts/build-web-instruments.sh

import { readFileSync } from "node:fs";
import { loadControlShell } from "./control-shell.js";
import { createAgentModule, loadAgentRuntime } from "./agent-module.js";
import { createModelPort, gatewayAddress } from "./agent-model-port.js";
import { agentOffer, agentTelemetry } from "./agent-inputs.js";
import { CONTROL_ACTION } from "./wire.js";

const controlWasm = readFileSync(new URL("./control-runtime_bg.wasm", import.meta.url));
const agentWasm = readFileSync(new URL("./agent-runtime_bg.wasm", import.meta.url));
const chart = readFileSync(new URL("./agent-chart.json", import.meta.url), "utf8");

let failures = 0;
function check(name, ok, got) {
  if (ok) {
    console.log(`ok   - ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL - ${name}${got === undefined ? "" : ` (got ${JSON.stringify(got)})`}`);
  }
}

const SESSION = { mode: "quad-pilot", connected: true, inputLost: false, nowMs: 100_000 };
const ON_THE_PAD = {
  posNed: Float64Array.of(0, 0, 0),
  velNed: Float64Array.of(0, 0, 0),
  quatWxyz: Float64Array.of(1, 0, 0, 0),
  armState: 1,
};
const reply = (directive) => JSON.stringify({ directive, probabilities: {}, model_ms: 1 });

async function liveShell() {
  const shell = await loadControlShell(controlWasm);
  shell.beginSession();
  shell.authorityEvent("motion", "grant", { generation: 1n });
  shell.beginControlRun();
  return shell;
}

/** One module over a live control runtime, with a scripted model. */
async function harness({ answers = {}, mode = "quad-pilot", offer = null } = {}) {
  const shell = await liveShell();
  const log = [];
  let clock = 0;
  const module = createAgentModule({
    loadRuntime: () => loadAgentRuntime(agentWasm),
    loadChart: async () => chart,
    modelPort: {
      baseUrl: "stub",
      declaration: async () => JSON.stringify({ ready: true, adapter: "stub/1", model: "scripted" }),
      ask: async (line) => answers[JSON.parse(line).message] ?? JSON.stringify({ error: "no script" }),
    },
    controlShell: () => shell,
    offer: () => offer ?? { maxLinearMps: 5, disarmOffered: true },
    telemetry: () => ON_THE_PAD,
    flightMode: () => mode,
    log: (line) => log.push(line),
    now: () => clock,
  });
  const advance = (ms) => {
    clock += ms;
    return clock;
  };
  /** One control tick, as the control loop runs it. */
  const tick = (pad = null) => {
    const input = shell.agentEngaged ? module.tick(advance(33)) : null;
    const plan = input ? shell.tickFromAgent(pad, input, SESSION) : shell.tickFromKeys(SESSION);
    if (plan.agentOverridden) module.overridden();
    return plan;
  };
  return { shell, module, log, tick };
}

// An engage goes through the control runtime, and the announcement then
// names the agent.
{
  const { shell, module, tick } = await harness();
  check("the module starts released", module.status().engaged === false);
  const refusal = await module.engage();
  check("an engage with an offer and a velocity mode is accepted", refusal === null, refusal);
  check("the control runtime has the agent as its source", shell.agentEngaged === true);
  for (let i = 0; i < 3; i += 1) tick();
  check("the announcement names the agent", shell.deviceLabel() === "automation.intent-pilot/v1", shell.deviceLabel());
  module.release();
  check("a release returns the source to the operator", shell.agentEngaged === false);
  check("the status follows the release", module.status().engaged === false);
}

// A model reply that passes the checks is flown: the agent asks for an arm
// through the typed arm edge of the control runtime.
{
  const { module, tick } = await harness({
    answers: { "Cleared for takeoff.": reply({ kind: "takeoff" }) },
  });
  await module.engage();
  for (let i = 0; i < 3; i += 1) tick();
  await module.submit("Cleared for takeoff.");
  const flown = module.status().history.some((entry) => entry.kind === "flown");
  check("the takeoff directive is flown", flown, module.status().history);
  let armed = false;
  for (let i = 0; i < 6 && !armed; i += 1) armed = tick().arm;
  check("the agent's arm request becomes a typed arm", armed);
  check("the executor is in its arming phase", module.status().phase === "arming", module.status().phase);
}

// The grounding check stands between a model and the vehicle in the browser
// too: a permitted fix that the operator did not say is not flown.
{
  const { module, tick } = await harness({
    answers: {
      "Proceed direct ZULU and land.": reply({ kind: "direct_to", fix: "ALPHA", on_arrival: "land" }),
    },
  });
  await module.engage();
  for (let i = 0; i < 3; i += 1) tick();
  await module.submit("Proceed direct ZULU and land.");
  const last = module.status().history.at(-1);
  check("a reply that the message does not ground is refused", last.kind === "refused", last);
  check("a refused reply leaves the vehicle on the ground", module.status().phase === "idle", module.status().phase);
  await module.submit("What is the weather?");
  check("a gateway fault is a fault and not a directive", module.status().history.at(-1).kind === "fault");
}

// The first operator input wins, and the announcement then names the device
// that took control.
{
  const { shell, module, tick } = await harness();
  const PAD_ID = "DualSense Wireless Controller (STANDARD GAMEPAD Vendor: 054c Product: 0ce6)";
  shell.selectDevice(PAD_ID);
  const centred = { axes: [0, 0, 0, 0], buttons: [] };
  for (let i = 0; i < 3; i += 1) shell.tickFromPad(centred, SESSION);
  await module.engage();
  for (let i = 0; i < 3; i += 1) tick();
  const pad = { axes: [0, -1, 0, 0], buttons: [] };
  const taken = tick(pad);
  for (let i = 0; i < 3; i += 1) shell.tickFromPad(centred, SESSION);
  check("the pad that took control is the announced source", shell.deviceLabel() === "Sony DualSense", shell.deviceLabel());
  check("an operator stick releases the agent", taken.agentOverridden === true);
  check("the module follows the override", module.status().engaged === false);
  check("the control runtime has no agent source", shell.agentEngaged === false);
  const throttle = taken.motion ? taken.motion.throttle : 0;
  check("the deflection that took control is not flown", throttle === 0, throttle);
}

// An engage states why it cannot start.
{
  const attitude = await harness({ mode: "fpv" });
  check("attitude mode has no velocity law for the agent", (await attitude.module.engage()) !== null);
  const unoffered = await harness({ offer: { reason: "vehicle.motion advertises no arm action" } });
  const reason = await unoffered.module.engage();
  check("a scope with no arm action is refused by its reason", reason === "vehicle.motion advertises no arm action", reason);
  check("a refused engage leaves the operator as the source", unoffered.shell.agentEngaged === false);
  const released = await harness();
  await released.module.submit("Cleared for takeoff.");
  check("an instruction to a released agent is refused", released.module.status().history.at(-1).kind === "refused");
}

// The inputs come from this client's own advertisement and telemetry.
{
  const state = { connected: true, motionScope: "vehicle.motion", advertisedScopes: [] };
  check("no session is a reason", agentOffer({ ...state, connected: false }, 1n, null).reason !== undefined);
  check("no velocity intent is a reason", agentOffer(state, 1n, null).reason !== undefined);
  check("no arm action is a reason", agentOffer(state, 1n, { maxLinear: 5 }).reason !== undefined);
  const offered = agentOffer(
    {
      ...state,
      advertisedScopes: [
        { vehicleId: 1n, scope: "vehicle.motion", intents: [], actions: [{ action: CONTROL_ACTION.arm, modeTargets: [] }] },
      ],
    },
    1n,
    { maxLinear: 5 },
  );
  check("an arm action and a velocity intent are an offer", offered.maxLinearMps === 5 && offered.disarmOffered === false, offered);

  const snapshot = {
    validFlags: 1 | 4 | 8,
    attitude: { quat: { w: 1, x: 0, y: 0, z: 0 } },
    kinematics: { posNed: [1, 2, -3], velNed: [0, 0, 0] },
  };
  const sample = agentTelemetry(snapshot, { armState: 2, stale: false });
  check("a valid snapshot is a sample", sample !== null && sample.posNed[2] === -3 && sample.armState === 2, sample);
  check("a stale arm state is unknown", agentTelemetry(snapshot, { armState: 2, stale: true }).armState === 0);
  check("a snapshot with no valid position is no sample", agentTelemetry({ ...snapshot, validFlags: 1 | 8 }, null) === null);
  check("no snapshot is no sample", agentTelemetry(null, null) === null);
}

// The model port gives one line back for each outcome of the exchange.
{
  const calls = [];
  const port = createModelPort({
    baseUrl: "http://gateway:8098/",
    fetchImpl: async (url, init) => {
      calls.push({ url, method: init.method, body: init.body });
      return { ok: true, status: 200, text: async () => '{"directive":{"kind":"land"}}\n' };
    },
  });
  const line = await port.ask('{"message":"Cleared to land."}');
  check("a request line goes to the directive path", calls[0].url === "http://gateway:8098/v1/directive" && calls[0].method === "POST", calls[0]);
  check("the reply line comes back without a change", line === '{"directive":{"kind":"land"}}', line);

  const refused = createModelPort({
    baseUrl: "http://gateway:8098",
    fetchImpl: async () => ({ ok: false, status: 503, text: async () => "adapter is not running" }),
  });
  check("a gateway error is a fault line", JSON.parse(await refused.ask("{}")).error.includes("503"));
  const down = createModelPort({
    baseUrl: "http://gateway:8098",
    fetchImpl: async () => {
      throw new TypeError("fetch failed");
    },
  });
  check("an unreachable gateway is a fault line", JSON.parse(await down.declaration()).error.includes("not reachable"));
  check(
    "the gateway address comes from the agent parameter",
    gatewayAddress({ search: "?agent=http://model:9000", protocol: "http:", hostname: "viewer" }) === "http://model:9000",
  );
  check(
    "the default gateway is port 8098 of the page host",
    gatewayAddress({ search: "", protocol: "http:", hostname: "viewer" }) === "http://viewer:8098",
  );
}

if (failures > 0) {
  console.error(`${failures} agent-module check(s) failed`);
  process.exit(1);
}
console.log("agent-module: all checks passed");
