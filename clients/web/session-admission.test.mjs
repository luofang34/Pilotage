import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  ADMISSION_REJECTED_CLOSE_CODE,
  createJoinCredentials,
  failureAfterClose,
  joinCodeBytes,
  pilotageSessionUrl,
  transportCloseFailure,
} from "./session-admission.js";
import { createSessionBootstrap } from "./session-bootstrap.js";
import { createSessionTransport } from "./session-transport.js";
import { TransportSessionLifecycle } from "./transport-session.js";
import { resolveViewerElements } from "./viewer-elements.js";

const CERT = "cd".repeat(32);
const CODE = "a7".repeat(32);
const fixture = JSON.parse(readFileSync(
  new URL("./session-admission-fixture.json", import.meta.url), "utf8",
));
assert.equal(ADMISSION_REJECTED_CLOSE_CODE, fixture.rejectedCloseCode);
assert.equal(ADMISSION_REJECTED_CLOSE_CODE, 1346961409);
assert.deepEqual(joinCodeBytes(` ${CODE.toUpperCase()}\n`), new Uint8Array(32).fill(0xa7));
assert.equal(joinCodeBytes("").length, 0);
for (const invalid of ["aa", "g".repeat(64), "a".repeat(63), `${CODE}00`]) {
  assert.throws(() => joinCodeBytes(invalid), (error) => !error.message.includes(invalid));
}
assert.deepEqual(transportCloseFailure({ reason: "admission rejected" }), { phase: "transport" });
assert.equal(transportCloseFailure({ streamErrorCode: fixture.rejectedCloseCode }).phase, "rejected");
assert.equal(transportCloseFailure({ closeCode: 0 }).phase, "transport");
assert.equal(transportCloseFailure({ bootstrapRejected: true }).phase, "rejected");
assert.equal(pilotageSessionUrl("pilotage.local", "4433"), "https://pilotage.local:4433/pilotage");
assert.equal(pilotageSessionUrl("127.0.0.1", "4433"), "https://127.0.0.1:4433/pilotage");
for (const host of ["::1", "[::1]"]) {
  assert.equal(pilotageSessionUrl(host, "4433"), "https://[::1]:4433/pilotage");
}

const credentials = createJoinCredentials();
assert.deepEqual(credentials.forConnection("host#pin", CODE, true), joinCodeBytes(CODE));
assert.deepEqual(credentials.forConnection("host#pin", "different input", false), joinCodeBytes(CODE));
assert.throws(() => credentials.forConnection("other#pin", CODE, false));
assert.deepEqual(credentials.forConnection("other#pin", "", true), new Uint8Array(0));

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((done, failed) => { resolve = done; reject = failed; });
  return { promise, resolve, reject };
}

function helloJoinToken(framed) {
  function fields(bytes) {
    const found = new Map();
    let at = 0;
    const varint = () => {
      let value = 0;
      let shift = 0;
      for (;;) {
        const byte = bytes[at++];
        value += (byte & 127) * 2 ** shift;
        if (!(byte & 128)) return value;
        shift += 7;
      }
    };
    while (at < bytes.length) {
      const tag = varint();
      if ((tag & 7) === 0) {
        found.set(tag >>> 3, varint());
      } else {
        assert.equal(tag & 7, 2);
        const len = varint();
        found.set(tag >>> 3, bytes.slice(at, at + len));
        at += len;
      }
    }
    return found;
  }
  assert.ok(framed[0] < 128, "this Hello has a one-byte length prefix");
  const hello = fields(framed.slice(1)).get(6);
  assert.ok(hello, "ClientHello envelope field");
  return fields(hello).get(3) ?? new Uint8Array(0);
}

function harness(code = CODE) {
  const original = Object.fromEntries(
    ["window", "document", "fetch", "WebTransport", "setTimeout", "clearTimeout"]
      .map((name) => [name, globalThis[name]]),
  );
  const welcome = JSON.parse(readFileSync(
    new URL("../web-control/server-welcome-fixture.json", import.meta.url), "utf8",
  ));
  let created = deferred();
  const telemetry = deferred();
  const failed = deferred();
  const halted = deferred();
  const closeWait = deferred();
  const timers = new Set();
  const state = {};
  const instances = [];
  const logs = [];
  const log = (message) => {
    logs.push(message);
    if (message.startsWith("connection cannot be auto-recovered")) halted.resolve();
  };
  const elements = {
    host: { value: "pilotage.local" },
    port: { value: "4433" },
    certHash: { value: CERT },
    joinCode: { value: code },
    resumeBtn: { hidden: true },
  };
  const els = resolveViewerElements({
    getElementById: (id) => elements[id],
    querySelectorAll: () => [],
  });
  class Transport {
    constructor(url, options) {
      instances.push(this);
      this.url = url;
      this.options = options;
      this.ready = Promise.resolve();
      this.closure = deferred();
      this.closed = this.closure.promise;
      this.reading = deferred();
      this.received = deferred();
      this.writes = [];
      created.resolve(this);
    }
    async createBidirectionalStream() {
      return {
        writable: { getWriter: () => ({
          write: async (bytes) => { this.writes.push(bytes); },
          abort: async () => {},
        }) },
        readable: { getReader: () => ({
          read: () => { this.reading.resolve(); return this.received.promise; },
          cancel: async () => { this.received.resolve({ done: true }); },
        }) },
      };
    }
    close() {
      this.closure.resolve({ closeCode: 0 });
      this.received.resolve({ done: true });
    }
    welcome() {
      this.received.resolve({
        done: false, value: Uint8Array.from(Buffer.from(welcome.envelopeHex, "hex")),
      });
    }
  }
  Object.assign(globalThis, {
    window: { location: { search: `?joinToken=${"ff".repeat(32)}` } },
    document: { visibilityState: "visible" },
    WebTransport: Transport,
    fetch: async () => ({
      ok: true,
      json: async () => ({
        host: els.host.value, port: 4433, certHash: CERT, joinToken: "ff".repeat(32),
      }),
    }),
    setTimeout: (callback, delay) => {
      const timer = { callback, delay };
      timers.add(timer);
      if (delay === 1000) closeWait.resolve();
      return timer;
    },
    clearTimeout: (timer) => timers.delete(timer),
  });
  const sessions = new TransportSessionLifecycle();
  const bootstrap = createSessionBootstrap({
    state, transportSessions: sessions, motionScope: "vehicle.motion",
    log, velocityCapabilityFor: () => true,
    refreshFeelModeControl() {},
  });
  const transport = createSessionTransport({
    state, els, transportSessions: sessions, bootstrap,
    controlGate: { reset() {}, isLatched: () => true },
    releaseTracker: { settled: async () => {}, abandon() {} },
    motionScope: "vehicle.motion", log,
    surface: { noControlLease() {} },
    readout: {
      resetVideoDiagnostics() {},
      retireSessionPresentation: (phase) => { if (phase === "failed") failed.resolve(); },
      beginTelemetrySession: () => telemetry.resolve(),
      acceptIncomingUniStreams: async () => {}, readTelemetryDatagrams: async () => {},
    },
    control: {
      authorityFor: () => ({ granted: false }),
      startSuspendedPressWatch() {}, runSessionStreamReader: async () => {},
    },
  });
  return {
    state, els, logs, transport, timers, closeWait, telemetry, failed, halted, instances,
    async nextTransport() {
      const instance = await created.promise;
      created = deferred();
      return instance;
    },
    restore() {
      transport.reconnect.stop();
      sessions.close(sessions.currentToken());
      Object.assign(globalThis, original);
    },
  };
}

for (const [field, value] of [
  ["host", ""], ["port", ""], ["certHash", ""],
  ["certHash", "a".repeat(63)], ["certHash", "g".repeat(64)], ["host", "pilotage.local/path"],
]) {
  const h = harness();
  try {
    const valid = h.els[field].value;
    h.els[field].value = value;
    assert.deepEqual(await h.transport.connect(), { ok: false, failure: { phase: "construct" } });
    assert.equal(h.instances.length, 0, `${field}: invalid target cannot create a transport`);
    assert.ok(!h.logs.join("\n").includes(CODE));
    h.els[field].value = valid;
    h.transport.reconnect.requestConnect();
    const transport = await h.nextTransport();
    await transport.reading.promise;
    assert.deepEqual(helloJoinToken(transport.writes[0]), joinCodeBytes(CODE));
    transport.welcome();
    await h.telemetry.promise;
    assert.equal(h.state.connected, true);
  } finally {
    h.restore();
  }
}

for (const code of ["", CODE]) {
  const h = harness(code);
  try {
    h.transport.applyUrlParams();
    await h.transport.refreshSessionConfig();
    assert.equal(h.els.joinCode.value, code, "URL and discovery cannot replace the credential");
    h.transport.reconnect.requestConnect();
    const transport = await h.nextTransport();
    await transport.reading.promise;
    assert.deepEqual(helloJoinToken(transport.writes[0]), joinCodeBytes(code));
    assert.ok(!transport.url.includes(CODE));
    assert.ok(!JSON.stringify(transport.options).includes(CODE));
    transport.welcome();
    await h.telemetry.promise;
    assert.equal(h.state.connected, true);
    assert.ok(!h.logs.join("\n").includes(CODE));
  } finally {
    h.restore();
  }
}

{
  const h = harness();
  try {
    h.transport.reconnect.requestConnect();
    const transport = await h.nextTransport();
    await transport.reading.promise;
    const error = new Error("bootstrap refused");
    error.bootstrapRejected = true;
    transport.received.reject(error);
    await h.halted.promise;
    assert.equal(h.transport.reconnect.snapshot().halted, true);
    assert.equal(h.timers.size, 0, "typed bootstrap refusal cannot become a transport retry");
  } finally {
    h.restore();
  }
}

{
  const h = harness();
  try {
    h.transport.reconnect.requestConnect();
    const refused = await h.nextTransport();
    await refused.reading.promise;
    refused.received.resolve({ done: true });
    await h.closeWait.promise;
    refused.closure.resolve({ closeCode: ADMISSION_REJECTED_CLOSE_CODE });
    await h.failed.promise;
    assert.equal(h.transport.reconnect.snapshot().halted, true);
    assert.equal(h.timers.size, 0);
    h.transport.reconnect.notifyVisible();
    assert.equal(h.timers.size, 0, "visibility does not retry a refused credential");
    assert.ok(h.logs.some((message) => message.includes("Join code was rejected")));
    assert.ok(!h.logs.join("\n").includes(CODE));

    h.els.joinCode.value = "b9".repeat(32);
    h.transport.reconnect.requestConnect();
    const accepted = await h.nextTransport();
    await accepted.reading.promise;
    assert.deepEqual(helloJoinToken(accepted.writes[0]), new Uint8Array(32).fill(0xb9));
    accepted.welcome();
    await h.telemetry.promise;
    assert.equal(h.state.connected, true);
    assert.equal(h.transport.reconnect.snapshot().halted, false);
  } finally {
    h.restore();
  }
}

{
  const closed = deferred();
  let timeout;
  let cancelled = false;
  const pending = failureAfterClose(closed.promise, { phase: "transport" }, (callback) => {
    timeout = callback;
    return 1;
  }, () => { cancelled = true; });
  timeout();
  assert.deepEqual(await pending, { phase: "transport" });
  assert.equal(cancelled, true, "a stream-only EOF has a bounded wait");
  const rejected = transportCloseFailure({ bootstrapRejected: true });
  assert.equal((await failureAfterClose(Promise.resolve({ phase: "transport" }), rejected)).phase,
    "rejected", "a bootstrap rejection remains terminal even if the session closes normally");
}
console.log("ok - session admission, credential isolation, and application-close recovery");
