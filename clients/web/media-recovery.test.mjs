import assert from "node:assert/strict";

import { MediaRecoveryGate } from "./media-recovery.js";

const gate = new MediaRecoveryGate(2000);
assert.equal(gate.shouldRequest(false, 10_000), false);
assert.equal(gate.shouldRequest(true, 10_000), true);
assert.equal(gate.shouldRequest(true, 11_999), false);
assert.equal(gate.shouldRequest(true, 12_000), true);
gate.notePaintedFrame();
assert.equal(gate.shouldRequest(true, 12_001), true);
gate.reset();
assert.equal(gate.shouldRequest(true, 12_002), true);

console.log("media recovery retry policy passed");
