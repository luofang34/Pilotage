// The explicit lease-release tracker (CTRL-04, #147) under fake time: the
// acknowledgement settles the release, the bound settles a lost one (the
// host watchdog covers from there), and an immediate reconnect awaits
// settlement so a fresh LeaseRequest cannot race into AlreadyHeld.
//
// Run: node clients/web/lease-release.test.mjs

import { createReleaseTracker } from "./lease-release.js";

let failures = 0;
function check(name, cond) {
  if (cond) {
    console.log(`ok   - ${name}`);
  } else {
    console.error(`FAIL - ${name}`);
    failures += 1;
  }
}

function fakeClock() {
  let seq = 0;
  const timers = new Map();
  return {
    schedule: (cb, ms) => {
      seq += 1;
      timers.set(seq, { cb, ms });
      return seq;
    },
    cancel: (handle) => timers.delete(handle),
    fire: () => {
      for (const [handle, { cb }] of [...timers]) {
        timers.delete(handle);
        cb();
      }
    },
    armed: () => timers.size,
  };
}

// --- acknowledgement settles the release ------------------------------------
{
  const clock = fakeClock();
  const tracker = createReleaseTracker({ timeoutMs: 1200, schedule: clock.schedule, cancel: clock.cancel });
  const settled = tracker.begin();
  check("release is pending after begin", tracker.isPending() === true);
  tracker.acknowledge();
  check("ack resolves the release", (await settled) === "acknowledged");
  check("ack cancels the timeout", clock.armed() === 0);
  check("nothing pending after ack", tracker.isPending() === false);
}

// --- a lost acknowledgement settles at the bound (watchdog covers) ----------
{
  const clock = fakeClock();
  const tracker = createReleaseTracker({ timeoutMs: 1200, schedule: clock.schedule, cancel: clock.cancel });
  const settled = tracker.begin();
  clock.fire();
  check("the bound settles a lost ack as timeout", (await settled) === "timeout");
}

// --- transport death abandons the release -----------------------------------
{
  const clock = fakeClock();
  const tracker = createReleaseTracker({ schedule: clock.schedule, cancel: clock.cancel });
  const settled = tracker.begin();
  tracker.abandon();
  check("transport death abandons", (await settled) === "abandoned");
}

// --- begin() is idempotent while pending -------------------------------------
{
  const clock = fakeClock();
  const tracker = createReleaseTracker({ schedule: clock.schedule, cancel: clock.cancel });
  const first = tracker.begin();
  const second = tracker.begin();
  check("a duplicate begin joins the pending release", first === second);
  tracker.acknowledge();
  await first;
}

// --- immediate reconnect awaits settlement (the AlreadyHeld race) ------------
{
  const clock = fakeClock();
  const tracker = createReleaseTracker({ schedule: clock.schedule, cancel: clock.cancel });
  const order = [];
  tracker.begin();

  // connect(manual) awaits settled() before requesting the lease.
  const connect = (async () => {
    await tracker.settled();
    order.push("lease-request");
  })();
  order.push("release-in-flight");
  tracker.acknowledge();
  await connect;
  check(
    "the lease request waits for the release to settle",
    order.join(",") === "release-in-flight,lease-request",
  );
}

// --- settled() passes immediately when idle -----------------------------------
{
  const tracker = createReleaseTracker();
  check("idle settles immediately", (await tracker.settled()) === "idle");
}

if (failures > 0) {
  console.error(`${failures} check(s) failed`);
  process.exit(1);
}
console.log("all lease-release checks passed");
