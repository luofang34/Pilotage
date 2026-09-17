// The agent client module in the browser (ADR-0042).
//
// The agent is one more input source of this client. It reads the telemetry
// that the instruments read, and its demand enters control through the control
// runtime, under the lease that this client holds. Each decision is in shared
// Rust: the agent runtime builds the model request, checks the model reply and
// runs the directive executor. This module moves text and numbers between the
// page, the model port, the agent runtime and the control runtime.

const VELOCITY_MODES = new Set(["quad-pilot", "quad-cruise"]);
const ARM_STATE_UNKNOWN = 0;

/** Loads the agent wasm runtime. */
export async function loadAgentRuntime(wasmUrl, options = {}) {
  const bindings = await (options.loadBindings ?? (() => import("./agent-runtime.js")))();
  await (options.initializeBindings ?? bindings.default)({ module_or_path: wasmUrl });
  return bindings.WebAgent;
}

/**
 * Creates the agent module.
 *
 * `offer()` gives what the motion scope advertises now, or a reason why the
 * agent cannot fly. `telemetry()` gives the newest operational-estimate
 * sample, or null. `controlShell()` gives the live control runtime, or null.
 */
export function createAgentModule({
  loadRuntime,
  loadChart,
  modelPort,
  controlShell,
  offer,
  telemetry,
  flightMode,
  log,
  now = () => performance.now(),
  onChange = () => {},
}) {
  let WebAgent = null;
  let agent = null;
  let startedMs = 0;
  let adapter = null;
  const history = [];

  const seconds = (nowMs) => (nowMs - startedMs) / 1000;

  function note(kind, text) {
    history.push({ kind, text });
    if (history.length > 40) history.shift();
    log(`agent: ${text}`);
    onChange();
  }

  function status() {
    return {
      engaged: agent !== null && controlShell()?.agentEngaged === true,
      phase: agent ? agent.phase() : "off",
      adapter,
      gateway: modelPort.baseUrl,
      history: [...history],
    };
  }

  /** Engages the agent as the input source. Returns a refusal reason, or null. */
  async function engage() {
    const shell = controlShell();
    if (!shell) return refuse("the control runtime is not ready");
    if (!VELOCITY_MODES.has(flightMode())) {
      return refuse("the agent flies the velocity law; select a Quad control mode");
    }
    const offered = offer();
    if (offered.reason) return refuse(offered.reason);
    try {
      WebAgent ??= await loadRuntime();
      const chart = await loadChart();
      agent = new WebAgent(chart, offered.maxLinearMps, offered.disarmOffered);
    } catch (error) {
      agent = null;
      return refuse(`the agent runtime did not start: ${error}`);
    }
    startedMs = now();
    if (!shell.engageAgent(WebAgent.profile_id())) {
      agent = null;
      return refuse("the control runtime refused the agent");
    }
    note("state", `engaged as ${WebAgent.profile_id()}; any control input takes control back`);
    void describeAdapter();
    return null;
  }

  function refuse(reason) {
    note("refused", `not engaged: ${reason}`);
    return reason;
  }

  async function describeAdapter() {
    const line = await modelPort.declaration();
    try {
      const declared = JSON.parse(line);
      adapter = declared.error ? null : `${declared.adapter} · ${declared.model}`;
      note("state", declared.error ? declared.error : `model: ${adapter}`);
    } catch {
      note("fault", "the model gateway declaration is not readable");
    }
  }

  /** Returns control to the operator's devices. The vehicle holds its place
   *  until the operator moves a control. */
  function release(reason = "released by the operator") {
    if (!agent) return;
    controlShell()?.disengageAgent();
    agent.free();
    agent = null;
    note("state", reason);
  }

  /** The control runtime saw an operator input and released the agent. */
  function overridden() {
    if (!agent) return;
    agent.free();
    agent = null;
    note("state", "an operator input took control from the agent");
  }

  /** One control tick: the agent's input for the control runtime, or null. */
  function tick(nowMs) {
    if (!agent) return null;
    const sample = telemetry();
    if (sample) {
      agent.observe(sample.posNed, sample.velNed, sample.quatWxyz, sample.armState ?? ARM_STATE_UNKNOWN);
    }
    const [roll, pitch, throttle, yaw, arm, disarm] = agent.step(seconds(nowMs));
    return { roll, pitch, throttle, yaw, arm: arm === 1, disarm: disarm === 1 };
  }

  /** Gives one operator message to the model and its reply to the agent. */
  async function submit(text) {
    const message = text.trim();
    if (!message) return;
    if (!agent) {
      note("refused", "engage the agent before you give an instruction");
      return;
    }
    note("operator", message);
    const asked = agent;
    let replyLine;
    try {
      replyLine = await modelPort.ask(asked.request(message));
    } catch (error) {
      replyLine = JSON.stringify({ error: String(error) });
    }
    // The agent can be released while the model reads. A reply for a released
    // agent has no vehicle to fly.
    if (agent !== asked) return;
    const decision = JSON.parse(agent.take_reply(message, replyLine, seconds(now())));
    if (decision.result === "flown") {
      note("flown", JSON.stringify(decision.directive));
    } else if (decision.result === "refused") {
      note("refused", `not flown: ${decision.reason}`);
    } else {
      note("fault", `model fault: ${decision.detail}`);
    }
  }

  function onActionResult(isArm, accepted) {
    agent?.on_action_result(isArm, accepted);
  }

  return { engage, release, overridden, tick, submit, onActionResult, status };
}
