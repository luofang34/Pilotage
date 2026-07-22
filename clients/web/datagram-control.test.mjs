import assert from "node:assert/strict";

import {
  acquireDatagramWriter,
  createDatagramRunStop,
  startDatagramControl,
} from "./datagram-control.js";
import { TransportSessionLifecycle } from "./transport-session.js";

function writer() {
  return {
    abortCount: 0,
    releaseCount: 0,
    abort() {
      this.abortCount += 1;
      return Promise.resolve();
    },
    releaseLock() {
      this.releaseCount += 1;
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
  assert.equal(lifecycle.currentToken(), token);
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
  assert.equal(lifecycle.currentToken(), null);
  assert.equal(activeWriter.releaseCount, 1);
}

function testInactiveSessionReleasesARefusedWriterLock() {
  const lifecycle = new TransportSessionLifecycle();
  const token = lifecycle.begin(transport());
  lifecycle.close(token);
  const refused = writer();
  const started = startDatagramControl({
    datagrams: { createWritable: () => ({ getWriter: () => refused }) },
    lifecycle,
    token,
    run: () => assert.fail("an inactive run must not start"),
    onError: () => assert.fail("an inactive run must not report a loop failure"),
  });
  assert.deepEqual(
    { ok: started.ok, reason: started.reason },
    { ok: false, reason: "inactive-session" },
  );
  assert.equal(refused.abortCount, 1);
  assert.equal(refused.releaseCount, 1);
}

async function testCompletedRunReleasesTheWriterForSameTransportReuse() {
  const lifecycle = new TransportSessionLifecycle();
  const token = lifecycle.begin(transport());
  let locked = false;
  const writers = [];
  const datagrams = {
    createWritable: () => ({
      getWriter() {
        if (locked) throw new Error("locked");
        locked = true;
        const acquired = writer();
        const releaseLock = acquired.releaseLock;
        acquired.releaseLock = () => {
          releaseLock.call(acquired);
          locked = false;
        };
        writers.push(acquired);
        return acquired;
      },
    }),
  };
  const first = startDatagramControl({
    datagrams,
    lifecycle,
    token,
    run: () => {},
    onError: () => assert.fail("the first run must not fail"),
  });
  assert.equal(first.ok, true);
  await first.completion;

  const second = startDatagramControl({
    datagrams,
    lifecycle,
    token,
    run: () => {},
    onError: () => assert.fail("the resumed run must not fail"),
  });
  assert.equal(second.ok, true);
  await second.completion;
  assert.equal(writers.length, 2);
  assert.deepEqual(writers.map((item) => item.releaseCount), [1, 1]);
}

async function testInputLossInterruptsPendingBackpressureWithoutClosingStream() {
  const stop = createDatagramRunStop();
  const pending = deferred();
  const ready = stop.waitFor(pending.promise);
  stop.stop();
  assert.equal(await ready, false);
  assert.equal(await stop.waitFor(Promise.resolve()), false);
  pending.resolve();
}

for (const test of [
  testPreferredSendStreamWins,
  testLegacySendStreamIsBoundedFallback,
  testMissingSendStreamFailsClosed,
  testWriterAcquisitionFailuresAreTyped,
  testSessionTeardownAbortsTheTrackedWriter,
  testInactiveSessionReleasesARefusedWriterLock,
  testCompletedRunReleasesTheWriterForSameTransportReuse,
  testInputLossInterruptsPendingBackpressureWithoutClosingStream,
]) {
  await test();
  console.log(`ok - ${test.name}`);
}
