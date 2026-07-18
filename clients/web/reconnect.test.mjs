// Lifecycle checks for the viewer's connection auto-recovery, driven by FAKE
// time (an injected scheduler) and a FAKE WebTransport (an injected `connect`
// returning programmed outcomes) — not only the pure decision helpers.
//
// Run: node clients/web/reconnect.test.mjs

import {
  reconnectDelayMs,
  classifyConnectFailure,
  createReconnectController,
} from "./reconnect.js";

let failures = 0;
function check(name, cond) {
  if (cond) {
    console.log(`ok   - ${name}`);
  } else {
    console.error(`FAIL - ${name}`);
    failures += 1;
  }
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

// A fake scheduler: records scheduled callbacks and lets the test fire the
// earliest one, so "time" advances only when the test says so.
function fakeScheduler() {
  let seq = 0;
  const timers = new Map();
  return {
    schedule(delayMs, cb) {
      seq += 1;
      timers.set(seq, { delayMs, cb });
      return seq;
    },
    cancel(id) {
      timers.delete(id);
    },
    pendingDelays() {
      return [...timers.values()].map((t) => t.delayMs);
    },
    count() {
      return timers.size;
    },
    async fireNext() {
      const entry = [...timers.entries()][0];
      if (!entry) return false;
      const [id, timer] = entry;
      timers.delete(id);
      timer.cb();
      await flush();
      return true;
    },
  };
}

// A fake WebTransport connect: returns programmed outcomes and records the
// `manual` flag of every attempt (the proof control is never auto-requested).
function fakeConnect(script) {
  const calls = [];
  let i = 0;
  return {
    calls,
    fn: async ({ manual }) => {
      calls.push(manual);
      const outcome = script[Math.min(i, script.length - 1)];
      i += 1;
      return outcome;
    },
  };
}

// ---- pure backoff -----------------------------------------------------------
{
  const opts = { baseMs: 1000, maxMs: 15000, jitterRatio: 0 };
  check("first attempt waits the base delay", reconnectDelayMs(0, opts) === 1000);
  check("the delay doubles each attempt", reconnectDelayMs(2, opts) === 4000);
  check("the delay is capped", reconnectDelayMs(20, opts) === 15000);
  // With jitter, the value stays within the symmetric band around the base.
  const jittered = reconnectDelayMs(0, { baseMs: 1000, maxMs: 15000, jitterRatio: 0.25 }, 0.0);
  check("jitter=0 pulls to the low edge of the band", jittered === 750);
  const high = reconnectDelayMs(0, { baseMs: 1000, maxMs: 15000, jitterRatio: 0.25 }, 0.999);
  check("jitter≈1 pulls to the high edge of the band", high > 1000 && high <= 1250);
}

// ---- classification ---------------------------------------------------------
{
  check("a construct failure is non-retryable", classifyConnectFailure({ phase: "construct" }).retryable === false);
  check("an explicit rejection is non-retryable", classifyConnectFailure({ phase: "rejected", kind: "protocol" }).retryable === false);
  check("a transport drop is retryable", classifyConnectFailure({ phase: "transport" }).retryable === true);
  check("an unknown failure is retryable (do not strand the user)", classifyConnectFailure(undefined).retryable === true);
}

// A controller wired to fakes; `random: () => 0.5` yields the un-jittered value.
function harness(script, { visible = true, active = false } = {}) {
  const sched = fakeScheduler();
  const connect = fakeConnect(script);
  const logs = [];
  const controller = createReconnectController({
    connect: connect.fn,
    schedule: sched.schedule,
    cancel: sched.cancel,
    isVisible: () => visible,
    isActive: () => active,
    random: () => 0.5,
    log: (m) => logs.push(m),
    baseMs: 1000,
    maxMs: 15000,
  });
  return { sched, connect, logs, controller, setVisible: (v) => (visible = v) };
}

// ---- auto-reconnect restores the transport but never requests control -------
{
  const h = harness([{ ok: true }, { ok: true }]);
  h.controller.requestConnect();
  await flush();
  h.controller.notifyBootstrapComplete();
  h.controller.notifyDropped(); // unexpected drop

  check("a drop schedules exactly one reconnect", h.sched.count() === 1);
  await h.sched.fireNext();

  check("the reconnect ran", h.connect.calls.length === 2);
  check("the manual Connect requested control", h.connect.calls[0] === true);
  check(
    "the auto-reconnect did NOT request control (manual === false)",
    h.connect.calls[1] === false,
  );
}

// ---- backoff grows across failures, then resets on bootstrap ----------------
{
  const h = harness([
    { ok: false, failure: { phase: "transport" } },
    { ok: false, failure: { phase: "transport" } },
    { ok: false, failure: { phase: "transport" } },
    { ok: true },
  ]);
  h.controller.requestConnect();
  await flush(); // attempt #1 fails -> schedules with attempts=0 -> 1000
  check("first backoff is the base", h.sched.pendingDelays()[0] === 1000);
  await h.sched.fireNext(); // attempts=1, fails -> schedules 2000
  check("second backoff doubled", h.sched.pendingDelays()[0] === 2000);
  await h.sched.fireNext(); // attempts=2, fails -> schedules 4000
  check("third backoff doubled again", h.sched.pendingDelays()[0] === 4000);
  await h.sched.fireNext(); // attempts=3, this one SUCCEEDS -> no new timer
  check("a successful reconnect schedules nothing further", h.sched.count() === 0);

  // Bootstrap-complete (only reached on success) resets the backoff, so the
  // next unexpected drop starts again at the base delay.
  h.controller.notifyBootstrapComplete();
  h.controller.notifyDropped();
  check("bootstrap-complete reset the backoff to base", h.sched.pendingDelays()[0] === 1000);
}

// ---- a non-retryable failure stops auto-recovery ----------------------------
{
  const h = harness([{ ok: false, failure: { phase: "construct" } }]);
  h.controller.requestConnect();
  await flush();
  check("a non-retryable failure schedules no retry", h.sched.count() === 0);
  check(
    "the user is told to reconnect manually",
    h.logs.some((m) => m.includes("press Connect")),
  );
}

// ---- a hidden tab defers; becoming visible retries --------------------------
{
  const h = harness([{ ok: false, failure: { phase: "transport" } }, { ok: true }], {
    visible: false,
  });
  h.controller.requestConnect();
  await flush();
  check("a hidden tab schedules no reconnect", h.sched.count() === 0);
  h.setVisible(true);
  h.controller.notifyVisible();
  check("becoming visible schedules the deferred reconnect", h.sched.count() === 1);
}

// ---- stop() halts recovery --------------------------------------------------
{
  const h = harness([{ ok: false, failure: { phase: "transport" } }]);
  h.controller.requestConnect();
  await flush();
  check("a retry is pending before stop", h.sched.count() === 1);
  h.controller.stop();
  check("stop cancels the pending retry", h.sched.count() === 0);
  h.controller.notifyDropped();
  check("stop makes later drops inert", h.sched.count() === 0);
}


// --- a non-retryable failure LATCHES: nothing restarts auto-recovery --------
{
  const clock = fakeScheduler();
  const attempts = [];
  const controller = createReconnectController({
    connect: async ({ manual }) => {
      attempts.push(manual);
      return { ok: false, failure: { phase: "construct" } };
    },
    schedule: clock.schedule,
    cancel: clock.cancel,
    isVisible: () => true,
    isActive: () => false,
    random: () => 0,
    log: () => {},
  });
  controller.requestConnect();
  await flush();
  check("the construct failure halts auto-recovery", controller.snapshot().halted === true);
  const before = attempts.length;

  // A visibility change must NOT restart retries after the halt.
  controller.notifyVisible();
  await flush();
  check("visibilitychange cannot restart a halted recovery", controller.snapshot().pending === false);

  // Neither can a stray drop notification without a failure.
  controller.notifyDropped();
  await flush();
  check("a stray drop cannot restart a halted recovery", controller.snapshot().pending === false);
  check("no attempt ran after the halt", attempts.length === before);

  // Only an explicit Connect clears the halt — it attempts again, and a
  // repeat of the same non-retryable failure re-latches rather than
  // looping.
  controller.requestConnect();
  await flush();
  check("the explicit Connect attempted", attempts.length === before + 1);
  check("the repeated construct failure re-latches", controller.snapshot().halted === true);
  check("still nothing scheduled", controller.snapshot().pending === false);
}

if (failures > 0) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log("\nall reconnect lifecycle checks passed");
