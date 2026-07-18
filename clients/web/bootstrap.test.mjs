// The REAL bootstrap reader loop under fakes: proves an auto-reconnect
// (requestLease: false) emits no LeaseRequest from the loop that actually
// runs in main.js — not merely that a controller passes a flag — and that
// a stream that ends before ServerWelcome is distinguishable (a
// retryable untyped end whose position is still visible telemetry) from a post-welcome drop.
//
// Run: node clients/web/bootstrap.test.mjs

import { runBootstrapReader } from "./bootstrap.js";

let failures = 0;
function check(name, cond) {
  if (cond) {
    console.log(`ok   - ${name}`);
  } else {
    console.error(`FAIL - ${name}`);
    failures += 1;
  }
}

// One-byte-tag framing for the fake wire: [tag] per message, delivered in
// arbitrary chunk boundaries to exercise the buffering/consumed path.
const WELCOME = 1;
const LEASE_RESPONSE = 2;
const KIND = { [WELCOME]: "ServerWelcome", [LEASE_RESPONSE]: "LeaseResponse" };

function fakeDecode(bytes) {
  if (bytes.length === 0) return null;
  const kind = KIND[bytes[0]];
  return kind ? { kind, message: {}, consumed: 1 } : null;
}

function fakeReader(chunks) {
  const queue = [...chunks];
  return {
    read: async () =>
      queue.length > 0 ? { value: Uint8Array.from(queue.shift()), done: false } : { done: true },
  };
}

function harness({ chunks, requestLease, active = () => true, leaseOk = true }) {
  const seen = [];
  let leaseRequests = 0;
  const run = runBootstrapReader({
    reader: fakeReader(chunks),
    decode: fakeDecode,
    isActive: active,
    onMessage: (decoded) => seen.push(decoded.kind),
    requestLease,
    sendLeaseRequest: async () => {
      leaseRequests += 1;
      return leaseOk;
    },
  });
  return run.then((result) => ({ result, seen, leaseRequests: () => leaseRequests }));
}

// --- auto-reconnect: the real loop must emit no LeaseRequest ---------------
{
  const { result, seen, leaseRequests } = await harness({
    chunks: [[WELCOME]],
    requestLease: false,
  });
  check("auto-reconnect completes at ServerWelcome", result.completed === true);
  check("auto-reconnect is marked welcomed", result.welcomed === true);
  check("auto-reconnect emits ZERO LeaseRequests", leaseRequests() === 0);
  check("the welcome was delivered to the message sink", seen.includes("ServerWelcome"));
}

// --- manual connect: exactly one LeaseRequest, completes on the response ---
{
  const { result, seen, leaseRequests } = await harness({
    // Welcome split across chunk boundaries with the response following.
    chunks: [[], [WELCOME], [LEASE_RESPONSE]],
    requestLease: true,
  });
  check("manual connect completes on LeaseResponse", result.completed === true);
  check("manual connect emits exactly one LeaseRequest", leaseRequests() === 1);
  check("both messages delivered in order", seen.join(",") === "ServerWelcome,LeaseResponse");
}

// --- a duplicate welcome must not double-send the lease request ------------
{
  const { result, leaseRequests } = await harness({
    chunks: [[WELCOME, WELCOME], [LEASE_RESPONSE]],
    requestLease: true,
  });
  check("duplicate welcome still completes", result.completed === true);
  check("duplicate welcome sends one LeaseRequest", leaseRequests() === 1);
}

// --- close BEFORE welcome: distinguishable, but still retryable -------------
{
  const { result, leaseRequests } = await harness({
    chunks: [],
    requestLease: true,
  });
  check("pre-welcome close is incomplete", result.completed === false);
  check("pre-welcome close is NOT welcomed (telemetry only, still retryable)", result.welcomed === false);
  check("pre-welcome close never sent a lease", leaseRequests() === 0);
}

// --- close AFTER welcome: an ordinary transport drop ------------------------
{
  const { result } = await harness({
    chunks: [[WELCOME]],
    requestLease: true,
  });
  check("post-welcome close is incomplete", result.completed === false);
  check("post-welcome close IS welcomed (transport drop, retryable)", result.welcomed === true);
}

// --- a superseded session stops without completing --------------------------
{
  let reads = 0;
  const { result, leaseRequests } = await harness({
    chunks: [[WELCOME], [LEASE_RESPONSE]],
    requestLease: true,
    active: () => {
      reads += 1;
      return false;
    },
  });
  check("superseded session is incomplete", result.completed === false);
  check("superseded session stops at the first liveness check", reads === 1);
  check("superseded session never sent a lease", leaseRequests() === 0);
}

// --- a lease send superseded mid-write ends the bootstrap -------------------
{
  const { result, leaseRequests } = await harness({
    chunks: [[WELCOME], [LEASE_RESPONSE]],
    requestLease: true,
    leaseOk: false,
  });
  check("superseded lease send is incomplete", result.completed === false);
  check("superseded lease send happened once", leaseRequests() === 1);
}

if (failures > 0) {
  console.error(`${failures} check(s) failed`);
  process.exit(1);
}
console.log("all bootstrap reader checks passed");
