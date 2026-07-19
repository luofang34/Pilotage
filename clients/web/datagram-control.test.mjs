import assert from "node:assert/strict";

import { acquireDatagramWriter, startDatagramControl } from "./datagram-control.js";
import { TransportSessionLifecycle } from "./transport-session.js";

function writer() {
  return {
    abortCount: 0,
    abort() {
      this.abortCount += 1;
      return Promise.resolve();
    },
  };
}

function transport() {
  return {
    closeCount: 0,
    close() {
      this.closeCount += 1;
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

function testPreferredSendStreamWins() {
  const preferred = writer();
  let legacyReads = 0;
  const datagrams = {
    createWritable: () => ({ getWriter: () => preferred }),
    get writable() {
      legacyReads += 1;
      return { getWriter: () => writer() };
    },
  };
  const result = acquireDatagramWriter(datagrams);
  assert.equal(result.ok, true);
  assert.equal(result.writer, preferred);
  assert.equal(legacyReads, 0);
}

function testLegacySendStreamIsBoundedFallback() {
  const legacy = writer();
  const result = acquireDatagramWriter({ writable: { getWriter: () => legacy } });
  assert.equal(result.ok, true);
  assert.equal(result.writer, legacy);
}

function testMissingSendStreamFailsClosed() {
  assert.deepEqual(acquireDatagramWriter({}), {
    ok: false,
    reason: "send-stream-unavailable",
  });
}

function testWriterAcquisitionFailuresAreTyped() {
  const createFailure = acquireDatagramWriter({
    createWritable() {
      throw new Error("create failed");
    },
    writable: { getWriter: () => writer() },
  });
  assert.equal(createFailure.ok, false);
  assert.equal(createFailure.reason, "writer-acquisition-failed");

  const lockFailure = acquireDatagramWriter({
    createWritable: () => ({
      getWriter() {
        throw new Error("lock failed");
      },
    }),
  });
  assert.equal(lockFailure.ok, false);
  assert.equal(lockFailure.reason, "writer-acquisition-failed");
}

async function testSessionTeardownAbortsTheTrackedWriter() {
  const lifecycle = new TransportSessionLifecycle();
  const activeTransport = transport();
  const token = lifecycle.begin(activeTransport);
  const activeWriter = writer();
  const running = deferred();
  let runCount = 0;
  const started = startDatagramControl({
    datagrams: { createWritable: () => ({ getWriter: () => activeWriter }) },
    lifecycle,
    token,
    run: async () => {
      runCount += 1;
      await running.promise;
    },
    onError: () => assert.fail("the control loop must not fail"),
  });
  assert.equal(started.ok, true);
  await Promise.resolve();
  assert.equal(runCount, 1);

  lifecycle.close(token);
  assert.equal(activeWriter.abortCount, 1);
  assert.equal(activeTransport.closeCount, 1);
  running.resolve();
  await started.completion;
  assert.equal(lifecycle.isActive(token), false);
}

for (const test of [
  testPreferredSendStreamWins,
  testLegacySendStreamIsBoundedFallback,
  testMissingSendStreamFailsClosed,
  testWriterAcquisitionFailuresAreTyped,
  testSessionTeardownAbortsTheTrackedWriter,
]) {
  await test();
  console.log(`ok - ${test.name}`);
}
