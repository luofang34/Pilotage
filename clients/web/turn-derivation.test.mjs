// Direct feeder tests for the measurement-coherent turn derivation
// (DYN-01 review disposition): rates difference only between distinct
// accepted measurements of one stream over the measurement clock —
// never once per paint, never across a discontinuity.

import { MAX_TURN_DT_MS, MIN_TURN_DT_MS, TurnDerivation } from "./turn-derivation.js";

let failures = 0;
function check(name, cond) {
  if (cond) {
    console.log(`ok   - ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL - ${name}`);
  }
}

const DEG = Math.PI / 180;

function stamp(sequence, atMs, over = {}) {
  return {
    sourceId: 7n,
    sourceIncarnation: "0123456789abcdef0123456789abcdef",
    sourceEpoch: 1,
    sequence,
    acquiredAtNanos: BigInt(Math.round(atMs * 1e6)),
    clock: "simulation",
    ...over,
  };
}

// ---- repeated renders of one sample -----------------------------------------

{
  const d = new TurnDerivation();
  check("first sample seeds and declares nothing", d.update(0, 5, stamp(1, 0)) === null);
  const first = d.update(2 * DEG, 6, stamp(2, 100));
  check("second sample derives over the measurement interval", first !== null);
  check(
    "rate is delta over measurement dt, not render dt",
    Math.abs(first.turnRps - (2 * DEG) / 0.1) < 1e-9,
  );
  // Sixty render frames of the SAME measurement: the cached rate is
  // re-declared every time — never zero, never recomputed, never
  // amplified by frame timing.
  let stable = true;
  for (let frame = 0; frame < 60; frame += 1) {
    const again = d.update(2 * DEG, 6 + frame, stamp(2, 100));
    stable &&= again !== null && Math.abs(again.turnRps - first.turnRps) < 1e-12;
  }
  check("repeated renders re-declare the cached rate unchanged", stable);
  const aged = d.update(2 * DEG, 250, stamp(2, 100));
  check("re-declaration carries the measurement's current age", aged.ageMs === 250);
}

// ---- two samples across the 359/1 degree wrap --------------------------------

{
  const d = new TurnDerivation();
  d.update(359 * DEG, 5, stamp(1, 0));
  const wrapped = d.update(1 * DEG, 5, stamp(2, 100));
  check(
    "359 to 1 degrees differences as +2 degrees, never -358",
    wrapped !== null && Math.abs(wrapped.turnRps - (2 * DEG) / 0.1) < 1e-6,
  );
  const d2 = new TurnDerivation();
  d2.update(1 * DEG, 5, stamp(1, 0));
  const back = d2.update(359 * DEG, 5, stamp(2, 100));
  check(
    "1 to 359 degrees differences as -2 degrees",
    back !== null && Math.abs(back.turnRps + (2 * DEG) / 0.1) < 1e-6,
  );
}

// ---- duplicate and reordered samples -----------------------------------------

{
  const d = new TurnDerivation();
  d.update(0, 5, stamp(1, 0));
  const base = d.update(2 * DEG, 5, stamp(2, 100));
  const dup = d.update(2 * DEG, 5, stamp(2, 100));
  check(
    "an exact duplicate re-declares without advancing state",
    dup !== null && dup.turnRps === base.turnRps,
  );
  const reordered = d.update(50 * DEG, 5, stamp(1, 0));
  check(
    "a serially older sample is ignored entirely",
    reordered !== null && reordered.turnRps === base.turnRps,
  );
  const next = d.update(4 * DEG, 5, stamp(3, 200));
  check(
    "the state still differences from the newest accepted sample",
    next !== null && Math.abs(next.turnRps - (2 * DEG) / 0.1) < 1e-6,
  );
}

// ---- stream discontinuities ---------------------------------------------------

{
  const d = new TurnDerivation();
  d.update(0, 5, stamp(1, 0));
  d.update(2 * DEG, 5, stamp(2, 100));
  const crossed = d.update(90 * DEG, 5, stamp(3, 200, { sourceEpoch: 2 }));
  check("an epoch reset never differences across the boundary", crossed === null);
  const seeded = d.update(92 * DEG, 5, stamp(4, 300, { sourceEpoch: 2 }));
  check(
    "the new epoch derives only from its own samples",
    seeded !== null && Math.abs(seeded.turnRps - (2 * DEG) / 0.1) < 1e-6,
  );

  const incarnation = d.update(0, 5, stamp(5, 400, {
    sourceEpoch: 2,
    sourceIncarnation: "ffffffffffffffffffffffffffffffff",
  }));
  check("an incarnation change resets the derivation", incarnation === null);

  const clock = d.update(0, 5, stamp(6, 500, {
    sourceEpoch: 2,
    sourceIncarnation: "ffffffffffffffffffffffffffffffff",
    clock: "vehicle-boot",
  }));
  check("a clock-domain change resets the derivation", clock === null);
}

// ---- rapid reconnect ----------------------------------------------------------

{
  const d = new TurnDerivation();
  d.update(0, 5, stamp(1, 0));
  d.update(2 * DEG, 5, stamp(2, 100));
  // Session retirement between identical-looking streams: even a
  // plausible dt and the SAME stream key must not bridge the reset.
  d.reset();
  const afterReset = d.update(4 * DEG, 5, stamp(3, 200));
  check("after reset the first sample only seeds", afterReset === null);
  const rebuilt = d.update(6 * DEG, 5, stamp(4, 300));
  check(
    "reconnect derives from post-reset samples only",
    rebuilt !== null && Math.abs(rebuilt.turnRps - (2 * DEG) / 0.1) < 1e-6,
  );
}

// ---- out-of-bound measurement intervals ---------------------------------------

{
  const d = new TurnDerivation();
  d.update(0, 5, stamp(1, 0));
  const tooClose = d.update(1 * DEG, 5, stamp(2, MIN_TURN_DT_MS - 1));
  check("a pair closer than the minimum dt declares nothing", tooClose === null);
  const tooFar = d.update(2 * DEG, 5, stamp(3, MIN_TURN_DT_MS - 1 + MAX_TURN_DT_MS + 1));
  check("a pair farther than the maximum dt declares nothing", tooFar === null);
  const recovered = d.update(3 * DEG, 5, stamp(4, MIN_TURN_DT_MS + MAX_TURN_DT_MS + 100));
  check(
    "derivation recovers on the next in-bound pair",
    recovered !== null && Math.abs(recovered.turnRps - (1 * DEG) / 0.1) < 1e-6,
  );
  const gap = d.update(NaN, 5, null);
  check("a missing heading resets everything", gap === null);
}

if (failures > 0) {
  console.error(`${failures} check(s) failed`);
  process.exit(1);
}
console.log("all turn-derivation checks passed");
