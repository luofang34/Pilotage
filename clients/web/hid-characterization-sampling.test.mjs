import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import { BrowserHIDCharacterizationSampler } from "./hid-characterization-sampling.js";

const device = {
  vendor_id: 0x1209,
  product_id: 0x4f54,
  product: "RadioMaster Pocket",
};
const sourceAxes = [0, 1].map((source_index) => ({
  source_index,
  minimum: -1,
  maximum: 1,
  neutral_position: "centered",
}));
const gamepad = {
  id: "RadioMaster Pocket (Vendor: 1209 Product: 4f54)",
  index: 0,
  mapping: "",
  connected: true,
  timestamp: 1,
  axes: [0, 0],
};
const options = {
  device,
  gamepad,
  connectionEpoch: "gamepad-connection-1",
  sourceContractDigest: "a".repeat(64),
  sourceAxes,
};

const sampler = new BrowserHIDCharacterizationSampler(options);
sampler.beginIdle();
assert.equal(sampler.record(gamepad, options.connectionEpoch, 1000), true);
assert.equal(sampler.record(gamepad, options.connectionEpoch, 2000), false);
gamepad.timestamp = 5;
gamepad.axes = [0.01, 0];
assert.equal(sampler.record(gamepad, options.connectionEpoch, 5000), true);
sampler.endSegment();
sampler.beginMovement("roll");
gamepad.timestamp = 9;
gamepad.axes = [1, 0.01];
assert.equal(sampler.record(gamepad, options.connectionEpoch, 9000), true);
sampler.endSegment();

const capture = sampler.finish();
assert.equal(capture.schema_version, 1);
assert.equal(capture.source, "browser_gamepad");
assert.equal(capture.timestamp_source, "source");
assert.equal(capture.timing_observation, "polled_state_updates");
assert.equal(capture.deadzone_evidence.status, "unknown");
assert.equal(capture.device_instance_id, options.connectionEpoch);
assert.equal(capture.samples.length, 3, "animation polls do not become state updates");
assert.deepEqual(capture.samples.map((sample) => sample.source_at_us), [1000, 5000, 9000]);
assert.deepEqual(capture.segments, [
  { action: { kind: "idle" }, start_sequence: 0, end_sequence: 1 },
  {
    action: { kind: "movement", logical: "roll", positive_first: true },
    start_sequence: 2,
    end_sequence: 2,
  },
]);

const disconnected = { ...gamepad, connected: false };
const stableGamepad = { ...gamepad, connected: true };
const boundToDisconnectedObject = new BrowserHIDCharacterizationSampler({
  ...options,
  gamepad: stableGamepad,
  connectionEpoch: "gamepad-connection-2",
});
boundToDisconnectedObject.beginIdle();
assert.throws(
  () => boundToDisconnectedObject.record(disconnected, "gamepad-connection-2", 10_000),
  /connection instance changed|disconnected/,
);
assert.throws(
  () => boundToDisconnectedObject.record(stableGamepad, "reconnect", 10_000),
  /connection instance changed/,
);

const axisCountChanged = { ...gamepad, connected: true, axes: [0, 0] };
const countSampler = new BrowserHIDCharacterizationSampler({
  ...options,
  gamepad: axisCountChanged,
  connectionEpoch: "gamepad-connection-3",
});
countSampler.beginIdle();
axisCountChanged.axes = [0, 0, 0];
assert.throws(
  () => countSampler.record(axisCountChanged, "gamepad-connection-3", 10_000),
  /axis count changed/,
);

assert.throws(
  () => new BrowserHIDCharacterizationSampler({
    ...options,
    gamepad: { ...gamepad, id: "Unparseable Device" },
  }),
  /identity does not match/,
);
assert.throws(
  () => new BrowserHIDCharacterizationSampler({
    ...options,
    device: { vendor_id: 0, product_id: 0, product: "Generic Device" },
    gamepad: { ...gamepad, id: "Unparseable Device" },
  }),
  /wildcard device identity is not evidence/,
);
assert.throws(
  () => new BrowserHIDCharacterizationSampler({
    ...options,
    device: { ...options.device, product: "é".repeat(129) },
  }),
  /invalid device product name/,
);
assert.throws(
  () => new BrowserHIDCharacterizationSampler({
    ...options,
    device: { ...options.device, product: "\ud800" },
  }),
  /invalid device product name/,
);
assert.throws(
  () => new BrowserHIDCharacterizationSampler({
    ...options,
    device: { ...options.device, product: "\udc00" },
  }),
  /invalid device product name/,
);
assert.doesNotThrow(() => new BrowserHIDCharacterizationSampler({
  ...options,
  device: { ...options.device, product: "😀".repeat(64) },
}));
assert.doesNotThrow(() => new BrowserHIDCharacterizationSampler({
  ...options,
  connectionEpoch: "é".repeat(128),
}));
assert.throws(
  () => new BrowserHIDCharacterizationSampler({
    ...options,
    connectionEpoch: "é".repeat(129),
  }),
  /connectionEpoch must identify one connection/,
);
assert.throws(
  () => new BrowserHIDCharacterizationSampler({
    ...options,
    connectionEpoch: "\ud800",
  }),
  /connectionEpoch must identify one connection/,
);

const invalidLogicalSampler = new BrowserHIDCharacterizationSampler({
  ...options,
  connectionEpoch: "gamepad-connection-invalid-logical",
});
assert.throws(() => invalidLogicalSampler.beginMovement("\udc00"), /movement logical name is invalid/);

const oversizedSampler = new BrowserHIDCharacterizationSampler({
  ...options,
  gamepad: { ...gamepad, axes: [0, 0] },
  connectionEpoch: "gamepad-connection-oversized",
});
oversizedSampler.beginIdle();
assert.equal(
  oversizedSampler.record(oversizedSampler.binding.gamepad, "gamepad-connection-oversized", 11_000),
  true,
);
oversizedSampler.endSegment();
assert.throws(() => oversizedSampler.finish(1), /encoded capture byte limit reached/);

const fixtureBytes = (name) => readFileSync(
  new URL(`../../tools/hid-probe/fixtures/${name}`, import.meta.url),
  "utf8",
);
const fixture = (name) => JSON.parse(fixtureBytes(name));
const physicalTrace = fixture("synthetic-capture.json");
const expectedBrowserCapture = fixture("browser-capture.json");
const expectedBrowserBytes = fixtureBytes("browser-capture.json");
const goldenGamepad = {
  id: "Synthetic HID (Vendor: 1234 Product: 5678)",
  index: 7,
  mapping: "",
  connected: true,
  timestamp: 0,
  axes: [0, 0],
};
const goldenSampler = new BrowserHIDCharacterizationSampler({
  device: physicalTrace.device,
  gamepad: goldenGamepad,
  connectionEpoch: "browser-gamepad-synthetic-1",
  sourceContractDigest: expectedBrowserCapture.source_contract_digest,
  sourceAxes: expectedBrowserCapture.source_axes,
});
const scaleAxis = (value, span) => Number((value / span - 1).toFixed(6));
for (const segment of physicalTrace.segments) {
  if (segment.action.kind === "idle") goldenSampler.beginIdle();
  else goldenSampler.beginMovement(segment.action.logical === "roll" ? "slot0" : "slot1");
  for (let sequence = segment.start_sequence; sequence <= segment.end_sequence; sequence += 1) {
    const sample = physicalTrace.samples[sequence];
    goldenGamepad.timestamp = sample.observed_at_us / 1000;
    goldenGamepad.axes = [scaleAxis(sample.axes[0], 1000), scaleAxis(sample.axes[1], 500)];
    assert.equal(
      goldenSampler.record(goldenGamepad, "browser-gamepad-synthetic-1", sample.observed_at_us),
      true,
    );
  }
  goldenSampler.endSegment();
}
assert.equal(
  `${JSON.stringify(goldenSampler.finish())}\n`,
  expectedBrowserBytes,
  "the JavaScript sampler must produce the exact browser capture fixture",
);

console.log("browser HID characterization sampling passed");
