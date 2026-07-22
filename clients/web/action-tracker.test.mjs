// Pending-press discipline for typed discrete actions (CTRL-01): nonzero
// correlation ids, dedup-preserving repeats, conflict cancellation, answer
// timeout, and ack resolution — presses ride the reliable session stream
// exactly once.

import test from "node:test";
import assert from "node:assert/strict";
import {
  ACTION_TIMEOUT_MS,
  createActionTracker,
  enqueueAction,
  expirePending,
  pendingModeTarget,
  resolveAction,
} from "./action-tracker.js";

const ARM = 1;
const DISARM = 2;
const MODE_REQUEST = 3;
const FPV = 2;
const CAMERA = 1;
const SCOPE = "vehicle.motion";

test("a press stays pending until its ack settles it", () => {
  const tracker = createActionTracker();
  const id = enqueueAction(tracker, SCOPE, ARM, 0);
  assert.ok(id >= 1, "ids are nonzero — zero means no correlation on the wire");
  assert.deepEqual(expirePending(tracker, 43), [], "still inside the answer window");
  const entry = resolveAction(tracker, id);
  assert.equal(entry.action, ARM);
  assert.equal(tracker.pending.size, 0);
});

test("a repeated identical press keeps the original id", () => {
  // A fresh id would make the host execute the press twice; the repeat must
  // alias the in-flight one.
  const tracker = createActionTracker();
  const first = enqueueAction(tracker, SCOPE, ARM, 0);
  const second = enqueueAction(tracker, SCOPE, ARM, 5);
  assert.equal(second, first);
  assert.equal(tracker.pending.size, 1);
});

test("arm and disarm cancel each other", () => {
  // The newer press supersedes its inverse: only the disarm stays pending.
  const tracker = createActionTracker();
  enqueueAction(tracker, SCOPE, ARM, 0);
  const disarmId = enqueueAction(tracker, SCOPE, DISARM, 5, { cancels: [ARM] });
  assert.deepEqual([...tracker.pending.keys()], [disarmId]);
  assert.equal(tracker.pending.get(disarmId).action, DISARM);
});

test("a new mode request supersedes the pending one", () => {
  const tracker = createActionTracker();
  enqueueAction(tracker, SCOPE, MODE_REQUEST, 0, { modeTarget: FPV, cancels: [MODE_REQUEST] });
  assert.equal(pendingModeTarget(tracker, SCOPE, MODE_REQUEST), FPV);
  enqueueAction(tracker, SCOPE, MODE_REQUEST, 5, { modeTarget: CAMERA, cancels: [MODE_REQUEST] });
  assert.equal(pendingModeTarget(tracker, SCOPE, MODE_REQUEST), CAMERA);
  assert.equal(tracker.pending.size, 1);
});

test("an unanswered press expires after the answer window", () => {
  const tracker = createActionTracker();
  const id = enqueueAction(tracker, SCOPE, ARM, 0);
  const expired = expirePending(tracker, ACTION_TIMEOUT_MS + 1);
  assert.equal(expired.length, 1);
  assert.equal(expired[0].actionId, id);
  // Expired means gone: a late ack resolves nothing.
  assert.equal(resolveAction(tracker, id), null);
});

test("scopes are independent", () => {
  const tracker = createActionTracker();
  const armId = enqueueAction(tracker, SCOPE, ARM, 0);
  const gimbalId = enqueueAction(tracker, "vehicle.gimbal", 4, 0);
  assert.notEqual(armId, gimbalId);
  // Cancellation is scope-local: a gimbal press cannot cancel a motion one.
  enqueueAction(tracker, "vehicle.gimbal", DISARM, 1, { cancels: [ARM] });
  assert.ok(tracker.pending.has(armId), "the motion arm stays pending");
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
