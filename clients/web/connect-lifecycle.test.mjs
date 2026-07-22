// The connect/transport lifecycle under a racing blur (CTRL-04),
// executing the PRODUCTION orchestration: `negotiateSessionAuthority` —
// the exact module main.js runs — over the real bootstrap reader loop,
// the real control gate, and the real release tracker. A regression in
// the production ordering (gate reset timing, live lease probe,
// post-grant recheck) fails HERE, not only in a re-derivation.
//
// Run: node clients/web/connect-lifecycle.test.mjs

import { negotiateSessionAuthority } from "./connect-authority.js";
import { runBootstrapReader } from "./bootstrap.js";
import { createControlGate } from "./control-gate.js";
import { createReleaseTracker } from "./lease-release.js";
import { resumeGrantDecision, resumeSessionControl } from "./resume-control.js";

let failures = 0;
function check(name, cond) {
  if (cond) {
    console.log(`ok   - ${name}`);
  } else {
    console.error(`FAIL - ${name}`);
    failures += 1;
  }
}

const WELCOME = 1;
const LEASE_RESPONSE = 2;
const KIND = { [WELCOME]: "ServerWelcome", [LEASE_RESPONSE]: "LeaseResponse" };
const decode = (bytes) => {
  if (bytes.length === 0) return null;
  const kind = KIND[bytes[0]];
  return kind ? { kind, message: {}, consumed: 1 } : null;
};

// A reader whose chunks can run side effects as they are consumed — the
// seam where a "blur during an await of the connect" is injected.
function reader(steps) {
  const queue = [...steps];
  return {
    read: async () => {
      const step = queue.shift();
      if (!step) return { done: true };
      if (step.effect) step.effect();
      return { value: Uint8Array.from(step.bytes), done: false };
    },
  };
}

function deferred() {
  let resolve;
  const promise = new Promise((settle) => {
    resolve = settle;
  });
  return { promise, resolve };
}

// Drives the PRODUCTION orchestration over the real bootstrap loop, the
// way main.js wires it: openAndBootstrap runs the reader with the
// module's live probe and reports whether a lease was granted.
async function drive({ gate, tracker, steps, controlStarts = true }) {
  const events = [];
  let leaseRequests = 0;
  const session = await negotiateSessionAuthority({
    manual: true,
    gate,
    releases: tracker,
    openAndBootstrap: async (leaseProbe) => {
      const result = await runBootstrapReader({
        reader: reader(steps),
        decode,
        isActive: () => true,
        onMessage: () => {},
        requestLease: leaseProbe, // the module's live probe, unwrapped by nothing
        sendLeaseRequest: async () => {
          leaseRequests += 1;
          return true;
        },
      });
      return {
        completed: result.completed,
        leaseGranted: leaseRequests > 0,
        result,
      };
    },
    startControl: () => {
      events.push("start-control");
      return controlStarts;
    },
    controlUnavailable: () => events.push("control-unavailable"),
    releaseLease: () => events.push("release-now"),
    telemetryOnly: () => events.push("telemetry-only"),
  });
  return { session, events, leaseRequests };
}

// --- a granted lease with no datagram writer is surrendered -----------------
{
  const gate = createControlGate({ isFocused: () => true });
  const tracker = createReleaseTracker();
  const { events, leaseRequests } = await drive({
    gate,
    tracker,
    steps: [{ bytes: [WELCOME] }, { bytes: [LEASE_RESPONSE] }],
    controlStarts: false,
  });
  check("writer refusal follows one granted request", leaseRequests === 1);
  check(
    "writer refusal makes control unavailable and surrenders authority",
    events.join(",") === "start-control,control-unavailable",
  );
}

// --- a blur during the release-settlement await stays latched ---------------
{
  let focused = true;
  const gate = createControlGate({ isFocused: () => focused });
  const tracker = createReleaseTracker();
  // A pending release from the previous blur is still settling when the
  // user clicks Connect; a NEW blur lands during that await. The
  // production module resets the gate BEFORE this await, so the latch
  // must survive into the lease decision.
  const settling = tracker.begin();
  queueMicrotask(() => {
    focused = false;
    gate.latchInputLoss(); // the racing blur
    focused = true;
    tracker.acknowledge();
  });
  const { events, leaseRequests } = await drive({
    gate,
    tracker,
    steps: [{ bytes: [WELCOME] }, { bytes: [LEASE_RESPONSE] }],
  });
  await settling;
  check("a blur during the settlement await suppresses the lease request", leaseRequests === 0);
  check("the session is telemetry-only", events.join(",") === "telemetry-only");
}

// --- a blur during the bootstrap read await suppresses the request ----------
{
  const gate = createControlGate({ isFocused: () => true });
  const tracker = createReleaseTracker();
  const { session, events, leaseRequests } = await drive({
    gate,
    tracker,
    // The blur fires while the reader is between hello and welcome — an
    // await inside the real loop, after the module froze nothing.
    steps: [{ bytes: [WELCOME], effect: () => gate.latchInputLoss() }],
  });
  check("bootstrap completes telemetry-only", session.completed === true);
  check("the live probe suppressed the lease request", leaseRequests === 0);
  check("no control started", events.join(",") === "telemetry-only");
}

// --- a blur AFTER the request raced the grant: released immediately ---------
{
  const gate = createControlGate({ isFocused: () => true });
  const tracker = createReleaseTracker();
  const { events, leaseRequests } = await drive({
    gate,
    tracker,
    steps: [
      { bytes: [WELCOME] }, // probe passes; LeaseRequest emitted
      { bytes: [LEASE_RESPONSE], effect: () => gate.latchInputLoss() },
    ],
  });
  check("the request was emitted before the blur", leaseRequests === 1);
  check("the granted lease is released immediately", events.join(",") === "release-now");
}

// --- the clean path starts control exactly once ------------------------------
{
  const gate = createControlGate({ isFocused: () => true });
  const tracker = createReleaseTracker();
  const { session, events, leaseRequests } = await drive({
    gate,
    tracker,
    steps: [{ bytes: [WELCOME] }, { bytes: [LEASE_RESPONSE] }],
  });
  check("clean connect completes", session.completed === true);
  check("clean connect requests exactly once", leaseRequests === 1);
  check("clean connect starts control", events.join(",") === "start-control");
}

// --- a stale latch from a previous session re-arms on explicit connect ------
{
  const gate = createControlGate({ isFocused: () => true });
  gate.latchInputLoss(); // left over from the previous blur
  const tracker = createReleaseTracker();
  const { events, leaseRequests } = await drive({
    gate,
    tracker,
    steps: [{ bytes: [WELCOME] }, { bytes: [LEASE_RESPONSE] }],
  });
  check("an explicit connect re-arms a stale latch", leaseRequests === 1);
  check("control starts after the re-arm", events.join(",") === "start-control");
}

// --- same-session resume waits for release and writer settlement ------------
{
  const gate = createControlGate({ isFocused: () => true });
  gate.latchInputLoss();
  const tracker = createReleaseTracker();
  const release = tracker.begin();
  const stopped = deferred();
  const liveTransport = Object.freeze({ id: "same-transport" });
  let activeTransport = liveTransport;
  let requests = 0;
  let announcements = 0;
  let surrenders = 0;
  const resume = resumeSessionControl({
    gate,
    releases: tracker,
    controlSettled: () => stopped.promise,
    isSessionLive: () => activeTransport === liveTransport,
    announceActivation: () => {
      announcements += 1;
    },
    requestLeases: () => {
      requests += 1;
    },
    surrender: () => {
      surrenders += 1;
    },
  });
  await Promise.resolve();
  check("resume waits for the release acknowledgement", requests === 0);
  tracker.acknowledge();
  await release;
  await Promise.resolve();
  check("resume also waits for the prior datagram run", requests === 0);
  stopped.resolve();
  const result = await resume;
  check("resume requests authority on the existing transport", result.requested && requests === 1);
  check("resume re-announces before its request", announcements === 1);
  check("clean same-session resume does not surrender", surrenders === 0);
  check("the transport identity never changes", activeTransport === liveTransport);
}

// --- a blur during same-session release settlement aborts the request -------
{
  const gate = createControlGate({ isFocused: () => true });
  gate.latchInputLoss();
  const tracker = createReleaseTracker();
  const release = tracker.begin();
  let requests = 0;
  const resume = resumeSessionControl({
    gate,
    releases: tracker,
    controlSettled: () => Promise.resolve(),
    isSessionLive: () => true,
    announceActivation: () => {},
    requestLeases: () => {
      requests += 1;
    },
    surrender: () => {},
  });
  queueMicrotask(() => {
    gate.latchInputLoss();
    tracker.acknowledge();
  });
  await release;
  const result = await resume;
  check("a blur during release settlement leaves the gate latched", gate.isLatched());
  check("a raced blur sends no same-session request", !result.requested && requests === 0);
}

// --- a blur after the request surrenders any raced grant --------------------
{
  const gate = createControlGate({ isFocused: () => true });
  gate.latchInputLoss();
  const tracker = createReleaseTracker();
  let surrenders = 0;
  const result = await resumeSessionControl({
    gate,
    releases: tracker,
    controlSettled: () => Promise.resolve(),
    isSessionLive: () => true,
    announceActivation: () => {},
    requestLeases: () => gate.latchInputLoss(),
    surrender: () => {
      surrenders += 1;
    },
  });
  check("a post-request blur is detected", result.requested && result.interrupted);
  check("the interrupted request is surrendered", surrenders === 1);
  check(
    "a raced grant cannot start control",
    resumeGrantDecision({
      pending: true,
      granted: true,
      sessionLive: true,
      mayPublish: gate.mayPublish(),
    }) === "surrender",
  );
}

if (failures > 0) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log("\nall connect-lifecycle checks passed");
