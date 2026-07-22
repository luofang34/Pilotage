// Reliable-delivery discipline for typed discrete actions (CTRL-01):
// correlation ids, frame-riding retransmission, dedup-preserving repeats,
// conflict cancellation, timeout expiry, and ack resolution.

import test from "node:test";
import assert from "node:assert/strict";
import {
  ACTION_TIMEOUT_MS,
  createActionTracker,
  enqueueAction,
  frameActions,
  pendingModeTarget,
  resolveAction,
} from "./action-tracker.js";

const ARM = 1;
const DISARM = 2;
const MODE_REQUEST = 3;
const FPV = 2;
const CAMERA = 1;
const SCOPE = "vehicle.motion";

test("a press rides every frame until its ack settles it", () => {
  const tracker = createActionTracker();
  const id = enqueueAction(tracker, SCOPE, ARM, 0);
  assert.ok(id >= 1);
  // Two successive frames both carry the still-unacked press.
  for (const now of [10, 43]) {
    const { actions, expired } = frameActions(tracker, SCOPE, now);
    assert.deepEqual(actions, [{ action: ARM, actionId: id }]);
    assert.deepEqual(expired, []);
  }
  const entry = resolveAction(tracker, id);
  assert.equal(entry.action, ARM);
  assert.deepEqual(frameActions(tracker, SCOPE, 50).actions, []);
});

test("a repeated identical press keeps the original id", () => {
  // A fresh id would make the host execute the press twice; the repeat must
  // alias the in-flight one.
  const tracker = createActionTracker();
  const first = enqueueAction(tracker, SCOPE, ARM, 0);
  const second = enqueueAction(tracker, SCOPE, ARM, 5);
  assert.equal(second, first);
  assert.equal(frameActions(tracker, SCOPE, 10).actions.length, 1);
});

test("arm and disarm cancel each other", () => {
  // The host rejects a frame carrying both, so the newer press supersedes.
  const tracker = createActionTracker();
  enqueueAction(tracker, SCOPE, ARM, 0);
  const disarmId = enqueueAction(tracker, SCOPE, DISARM, 5, { cancels: [ARM] });
  const { actions } = frameActions(tracker, SCOPE, 10);
  assert.deepEqual(actions, [{ action: DISARM, actionId: disarmId }]);
});

test("a new mode request supersedes the pending one", () => {
  const tracker = createActionTracker();
  enqueueAction(tracker, SCOPE, MODE_REQUEST, 0, { modeTarget: FPV, cancels: [MODE_REQUEST] });
  assert.equal(pendingModeTarget(tracker, SCOPE, MODE_REQUEST), FPV);
  enqueueAction(tracker, SCOPE, MODE_REQUEST, 5, { modeTarget: CAMERA, cancels: [MODE_REQUEST] });
  const { actions } = frameActions(tracker, SCOPE, 10);
  assert.equal(actions.length, 1);
  assert.equal(actions[0].modeTarget, CAMERA);
});

test("an unanswered press expires after the retry window", () => {
  const tracker = createActionTracker();
  const id = enqueueAction(tracker, SCOPE, ARM, 0);
  const { actions, expired } = frameActions(tracker, SCOPE, ACTION_TIMEOUT_MS + 1);
  assert.deepEqual(actions, []);
  assert.equal(expired.length, 1);
  assert.equal(expired[0].actionId, id);
  // Expired means gone: no zombie retransmission on the next frame.
  assert.deepEqual(frameActions(tracker, SCOPE, ACTION_TIMEOUT_MS + 2).actions, []);
});

test("scopes are independent", () => {
  const tracker = createActionTracker();
  enqueueAction(tracker, SCOPE, ARM, 0);
  const gimbalId = enqueueAction(tracker, "vehicle.gimbal", 4, 0);
  const { actions } = frameActions(tracker, "vehicle.gimbal", 10);
  assert.deepEqual(actions, [{ action: 4, actionId: gimbalId }]);
  assert.equal(frameActions(tracker, SCOPE, 10).actions.length, 1);
});

test("resolving an unknown id is a harmless replay", () => {
  const tracker = createActionTracker();
  assert.equal(resolveAction(tracker, 999), null);
});

test("ids wrap within u32 and never mint 0", () => {
  const tracker = createActionTracker();
  tracker.nextId = 0xffffffff;
  const last = enqueueAction(tracker, SCOPE, ARM, 0);
  assert.equal(last, 0xffffffff);
  const wrapped = enqueueAction(tracker, SCOPE, DISARM, 0, { cancels: [ARM] });
  assert.equal(wrapped, 1);
});
