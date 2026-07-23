// The control loop must yield to MACROTASKS between iterations on EVERY
// path — including the publish-gated one. A path that re-enters the loop
// through only-microtask awaits (an immediately-ready datagram writer plus a
// bare `continue`) starves timers, rendering, and the WebTransport readers:
// the page freezes at 100% CPU and never recovers, because even the focus
// and click events that would open the gate ride the starved macrotask
// queue. The gated state is reachable un-latched: a launcher-opened window
// the operator never focused has `document.hasFocus() === false` with no
// blur event ever delivered, so `mayPublish()` is false while `isLatched()`
// is false.
//
// The check is event-based, not wall-clock: a zero-delay timer scheduled at
// loop start must fire before the loop completes its tick quota. In a
// microtask spin the timer can only run after the loop exits.
//
// Run: node clients/web/control-pacing.test.mjs

import { createControlLoop } from "./control-loop.js";

let failures = 0;
function check(name, cond) {
  if (cond) {
    console.log(`ok   - ${name}`);
  } else {
    console.error(`FAIL - ${name}`);
    failures += 1;
  }
}

// activeGamepad() probes the browser gamepad API; node has no navigator with
// gamepads, so give it an empty probe.
globalThis.navigator ??= {};

const TICK_QUOTA = 30;

async function runGatedLoop({ mayPublish }) {
  let ticks = 0;
  let active = true;
  const state = {
    connected: true,
    controlShell: null,
    selectedPadId: null,
    pendingReset: false,
    actionTracker: { pending: new Map(), nextId: 1 },
    controlCompletion: null,
    stopControlRun: null,
    motionScope: "vehicle.motion",
  };
  state.controlShell = {
    beginControlRun() {},
    tickFromKeys() {
      ticks += 1;
      if (ticks >= TICK_QUOTA) active = false;
      return { motion: null, gimbal: null, lease: null, motionLease: null };
    },
  };
  const loop = createControlLoop({
    state,
    els: { flightMode: { value: "rover" } },
    transportSessions: {
      isActive: () => active,
      trackWriter: () => true,
      untrackWriter() {},
      currentToken: () => 1,
    },
    controlGate: { isLatched: () => false, mayPublish: () => mayPublish, reset() {} },
    releaseTracker: {},
    vehicleId: "veh",
    motionScope: "vehicle.motion",
    directScope: "vehicle.motion.direct",
    gimbalScope: "vehicle.gimbal",
    lifecycleScope: "vehicle.lifecycle",
    frameRejectionUplinkIdle: () => false,
    // Fast cadence keeps the paced run short; pacing is what's under test,
    // not the interval width.
    controlHz: 200,
    log() {},
    surface: new Proxy({}, { get: () => () => {} }),
    updateControlReadout() {},
    reportSuppressedPresses() {},
    currentTelemetryHeading: () => null,
    lengthDelimit: (bytes) => bytes,
    maybeAnnounceProfileActivation() {},
    requestReconnect() {},
  });

  let ticksWhenTimerFired = null;
  setTimeout(() => {
    ticksWhenTimerFired = ticks;
  }, 0);

  const writer = { ready: Promise.resolve(), releaseLock() {}, write: async () => {} };
  const started = loop.startControlLoop(
    { datagrams: { createWritable: () => ({ getWriter: () => writer }) } },
    1,
  );
  check("the loop starts", started);
  await state.controlCompletion;
  // Let the zero-delay timer run if it has not yet.
  await new Promise((resolve) => setTimeout(resolve, 0));
  return { ticks, ticksWhenTimerFired };
}

// ---- the gated (mayPublish false, un-latched) path must still pace ---------
{
  const { ticks, ticksWhenTimerFired } = await runGatedLoop({ mayPublish: false });
  check(`the gated loop ran its quota (${ticks}/${TICK_QUOTA})`, ticks === TICK_QUOTA);
  check(
    "a zero-delay timer fires BEFORE the gated loop finishes its quota " +
      `(fired at tick ${ticksWhenTimerFired})`,
    ticksWhenTimerFired !== null && ticksWhenTimerFired < TICK_QUOTA,
  );
}

// ---- positive control: the publishing path paces the same way --------------
{
  const { ticks, ticksWhenTimerFired } = await runGatedLoop({ mayPublish: true });
  check(`the publishing loop ran its quota (${ticks}/${TICK_QUOTA})`, ticks === TICK_QUOTA);
  check(
    "a zero-delay timer interleaves with the publishing loop " +
      `(fired at tick ${ticksWhenTimerFired})`,
    ticksWhenTimerFired !== null && ticksWhenTimerFired < TICK_QUOTA,
  );
}

if (failures > 0) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log("\nall control-pacing checks passed");
