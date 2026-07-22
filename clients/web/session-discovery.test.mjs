import assert from "node:assert/strict";

import {
  applySessionConfig,
  validSessionConfig,
  whenVisible,
} from "./session-discovery.js";

const CERT = "ab".repeat(32);

function testManifestValidationFailsClosed() {
  assert.deepEqual(validSessionConfig({ host: "127.0.0.1", port: 4433, certHash: CERT }), {
    host: "127.0.0.1",
    port: "4433",
    certHash: CERT,
  });
  // Every rejection is total: a partial manifest never half-applies.
  assert.equal(validSessionConfig(null), null);
  assert.equal(validSessionConfig({ port: 4433, certHash: CERT }), null);
  assert.equal(validSessionConfig({ host: " ", port: 4433, certHash: CERT }), null);
  assert.equal(validSessionConfig({ host: "127.0.0.1", port: 0, certHash: CERT }), null);
  assert.equal(validSessionConfig({ host: "127.0.0.1", port: 70000, certHash: CERT }), null);
  assert.equal(validSessionConfig({ host: "127.0.0.1", port: "4433", certHash: CERT }), null);
  assert.equal(validSessionConfig({ host: "127.0.0.1", port: 4433, certHash: "nothex" }), null);
  assert.equal(
    validSessionConfig({ host: "127.0.0.1", port: 4433, certHash: CERT.slice(2) }),
    null,
  );
}
testManifestValidationFailsClosed();
console.log("ok - testManifestValidationFailsClosed");

function testApplyReportsOnlyRealChanges() {
  const els = {
    host: { value: "127.0.0.1" },
    port: { value: "4433" },
    certHash: { value: CERT },
  };
  const same = { host: "127.0.0.1", port: "4433", certHash: CERT };
  assert.equal(applySessionConfig(els, same), false, "identical config is not a change");
  const fresh = { host: "127.0.0.1", port: "4433", certHash: "cd".repeat(32) };
  assert.equal(applySessionConfig(els, fresh), true, "a new cert is a change");
  assert.equal(els.certHash.value, "cd".repeat(32));
}
testApplyReportsOnlyRealChanges();
console.log("ok - testApplyReportsOnlyRealChanges");

function testAutoconnectWaitsForVisibility() {
  // Already visible: begin runs synchronously.
  let began = 0;
  whenVisible({ visibilityState: "visible" }, () => {
    began += 1;
  });
  assert.equal(began, 1);

  // Hidden: begin waits for the first visible transition, then detaches.
  const listeners = [];
  const doc = {
    visibilityState: "hidden",
    addEventListener: (kind, fn) => listeners.push([kind, fn]),
    removeEventListener: (kind, fn) => {
      const at = listeners.findIndex(([k, f]) => k === kind && f === fn);
      if (at >= 0) listeners.splice(at, 1);
    },
  };
  whenVisible(doc, () => {
    began += 1;
  });
  assert.equal(began, 1, "no connect while hidden");
  assert.equal(listeners.length, 1);
  // A change that lands still-hidden must not begin.
  listeners[0][1]();
  assert.equal(began, 1);
  doc.visibilityState = "visible";
  listeners[0][1]();
  assert.equal(began, 2, "begins on the first visible transition");
  assert.equal(listeners.length, 0, "the listener detaches after beginning");
}
testAutoconnectWaitsForVisibility();
console.log("ok - testAutoconnectWaitsForVisibility");
