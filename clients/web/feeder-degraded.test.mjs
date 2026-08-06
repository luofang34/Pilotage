// The wasm-absent degraded contract (#252): with the instrument wasm
// unavailable every feeder lane fails closed — nothing admitted, no
// substitute values, a malformed verdict for every stamp — and no lane
// throws from the middle of the telemetry loop. The env knob must be
// set before the module graph loads, so the imports are dynamic.

process.env.PILOTAGE_FEEDER_WASM_DISABLE = "1";

const assert = await import("node:assert/strict").then((m) => m.default);
const { bindings } = await import("./feeder-wasm.js");
const { AvionicsIngress, COHERENCE, FcStateTracker, NavGuidanceTracker, ROLE, stampFaultForRole } =
  await import("./telemetry-ingress.js");
const { TurnDerivation } = await import("./turn-derivation.js");

assert.equal(bindings, null);

// A perfectly well-shaped stamp: only the wasm could complete the
// verdict, so validation must fail closed, not pass half-checked.
const stamp = {
  role: 2,
  integrity: 2,
  sourceId: 7n,
  sourceIncarnation: "0123456789abcdef0123456789abcdef",
  sourceEpoch: 1,
  sequence: 1,
  acquiredAtNanos: 1_000_000n,
  clock: 2,
};
assert.deepEqual(stampFaultForRole(stamp, ROLE.SIMULATION_TRUTH), {
  field: "stamp",
  rule: "malformed",
});

const gate = new AvionicsIngress({ vehicleId: 1n, maximumSkewNanos: 100n });
assert.equal(gate.ingest({ vehicleId: 1n, avionics: { attitudeStamp: stamp } }, 0), false);
const snapshot = gate.snapshot(0);
assert.equal(snapshot.generation, 0);
assert.equal(snapshot.attitude, null);
assert.equal(snapshot.quality, 2);
assert.equal(snapshot.coherence.status, COHERENCE.INSUFFICIENT);
const diagnostics = gate.diagnostics();
assert.equal(diagnostics.invalidStamps, 0);
assert.equal(diagnostics.lastRejectReason, null);
gate.generation = 5; // assignable surface must not throw without wasm
assert.equal(gate.snapshot(0).generation, 0);

const turn = new TurnDerivation();
assert.equal(turn.update(1.0, 5, stamp), null);
turn.reset();

assert.equal(new FcStateTracker().view(0), null);
assert.equal(new NavGuidanceTracker().snapshot(0), null);

console.log("feeder degraded-mode contract passed");
