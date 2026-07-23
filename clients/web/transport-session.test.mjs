import assert from "node:assert/strict";

import { TransportSessionLifecycle } from "./transport-session.js";

function fakeTransport(name) {
  return {
    name,
    closeCount: 0,
    close() {
      this.closeCount += 1;
    },
  };
}

function fakeReader() {
  return {
    cancelCount: 0,
    cancelReason: null,
    cancel(reason) {
      this.cancelCount += 1;
      this.cancelReason = reason;
      return Promise.resolve();
    },
  };
}

function fakeWriter() {
  return {
    abortCount: 0,
    abort() {
      this.abortCount += 1;
      return Promise.resolve();
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

async function testInterleavedReplacementRejectsOldDataAndCallbacks() {
  const lifecycle = new TransportSessionLifecycle();
  const oldTransport = fakeTransport("old");
  const oldToken = lifecycle.begin(oldTransport);
  const oldReader = fakeReader();
  const oldWriter = fakeWriter();
  lifecycle.trackReader(oldToken, oldReader);
  lifecycle.trackWriter(oldToken, oldWriter);

  const viewer = { connected: true, ingress: "old", status: "old ready" };
  const oldDatagram = deferred();
  const oldClosed = deferred();
  const delayedDatagram = oldDatagram.promise.then((value) => {
    lifecycle.runIfActive(oldToken, () => {
      viewer.ingress = value;
    });
  });
  const delayedClose = oldClosed.promise.then(() => {
    lifecycle.runIfActive(oldToken, () => {
      viewer.connected = false;
      viewer.status = "old closed";
    });
    lifecycle.retire(oldToken);
  });

  const newTransport = fakeTransport("new");
  const newToken = lifecycle.begin(newTransport);
  lifecycle.runIfActive(newToken, () => {
    viewer.connected = true;
    viewer.ingress = "new";
    viewer.status = "new ready";
  });

  assert.equal(oldTransport.closeCount, 1);
  assert.equal(oldReader.cancelCount, 1);
  assert.equal(oldReader.cancelReason?.kind, "session-replaced");
  assert.equal(oldWriter.abortCount, 1);
  oldDatagram.resolve("old datagram");
  oldClosed.resolve();
  await Promise.all([delayedDatagram, delayedClose]);
  assert.equal(lifecycle.isActive(newToken), true);
  assert.deepEqual(viewer, { connected: true, ingress: "new", status: "new ready" });

  assert.equal(
    lifecycle.runIfActive(newToken, () => {
      viewer.ingress = "new datagram";
    }),
    true,
  );
  assert.equal(viewer.ingress, "new datagram");
}

function testTokenIdentitySurvivesGenerationWrap() {
  const lifecycle = new TransportSessionLifecycle();
  lifecycle.generation = 0xffff_ffff;
  const oldToken = lifecycle.begin(fakeTransport("before wrap"));
  lifecycle.generation = 0xffff_ffff;
  const newToken = lifecycle.begin(fakeTransport("after wrap"));

  assert.equal(oldToken.generation, 0);
  assert.equal(newToken.generation, 0);
  assert.notEqual(oldToken, newToken);
  assert.equal(lifecycle.isActive(oldToken), false);
  assert.equal(lifecycle.isActive(newToken), true);
}

for (const test of [
  testInterleavedReplacementRejectsOldDataAndCallbacks,
  testTokenIdentitySurvivesGenerationWrap,
]) {
  await test();
  console.log(`ok - ${test.name}`);
}
