import assert from "node:assert/strict";

import { BrowserHIDCharacterizationSampler } from "./hid-characterization-sampling.js";

const sampler = new BrowserHIDCharacterizationSampler({
  vendor_id: 0x1209,
  product_id: 0x4f54,
  product: "RadioMaster Pocket",
});
sampler.beginIdle();
assert.equal(sampler.record({ timestamp: 1, axes: [0, 0] }, 1000), true);
assert.equal(sampler.record({ timestamp: 1, axes: [0, 0] }, 2000), false);
assert.equal(sampler.record({ timestamp: 5, axes: [0.01, 0] }, 5000), true);
sampler.endSegment();
sampler.beginMovement("roll");
assert.equal(sampler.record({ timestamp: 9, axes: [1, 0.01] }, 9000), true);
sampler.endSegment();

const capture = sampler.finish();
assert.equal(capture.schema_version, 1);
assert.equal(capture.source, "browser_gamepad");
assert.equal(capture.timestamp_source, "source");
assert.equal(capture.deadzone_evidence.status, "unknown");
assert.equal(capture.samples.length, 3, "animation polls do not become HID reports");
assert.deepEqual(capture.samples.map((sample) => sample.source_at_us), [1000, 5000, 9000]);
assert.deepEqual(capture.segments, [
  { action: { kind: "idle" }, start_sequence: 0, end_sequence: 1 },
  {
    action: { kind: "movement", logical: "roll", positive_first: true },
    start_sequence: 2,
    end_sequence: 2,
  },
]);

const paired = new BrowserHIDCharacterizationSampler(
  { vendor_id: 1, product_id: 2, product: null },
  {
    status: "observed",
    method: "paired_native_and_platform",
    sample_count: 20,
  },
);
paired.beginIdle();
paired.record({ timestamp: 1, axes: [0] }, 1);
paired.endSegment();
assert.equal(paired.finish().deadzone_evidence.status, "observed");

const withoutTimestamp = new BrowserHIDCharacterizationSampler({
  vendor_id: 1,
  product_id: 2,
});
withoutTimestamp.beginIdle();
assert.throws(
  () => withoutTimestamp.record({ axes: [0] }, 1),
  /Gamepad\.timestamp is required/,
);

console.log("browser HID characterization sampling passed");
