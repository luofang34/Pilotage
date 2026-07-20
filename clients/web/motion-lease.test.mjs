// The reliable-stream motion-lease transitions that main.js's session reader
// runs in production: release acknowledgement → motion response → generation
// installation, with the fence and terminal denial enforced off the wire.

import { initialMotionLease, advanceMotionLease } from "./motion-lease.js";

let failures = 0;
function check(name, ok) {
  if (ok) {
    console.log(`ok   - ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL - ${name}`);
  }
}

const released = { kind: "LeaseReleased", message: { released: true } };
const response = (granted, generation, reason) => ({
  kind: "LeaseResponse",
  message: { granted, generation, reason },
});

// Steady authority held on generation 7.
const held = { ...initialMotionLease(), granted: true, generation: 7n };

// A release drops authority and fences the generation we held (7).
const afterRelease = advanceMotionLease(held, released);
check("release drops the grant", afterRelease.granted === false);
check("release fences the held generation", afterRelease.fence === 7n);

// A regrant on a NEWER generation (8 > fence 7) installs and re-grants.
const afterGrant = advanceMotionLease(afterRelease, response(true, 8));
check("a newer grant is accepted", afterGrant.granted === true);
check("the fresh generation installs", afterGrant.generation === 8n);
check("a granted response is not denied", afterGrant.denied === false);

// A stale/replayed grant at or below the fence is rejected — control must not
// resume on the released generation.
const staleAtFence = advanceMotionLease(afterRelease, response(true, 7));
check("a grant AT the fence is rejected", staleAtFence.granted === false);
check("a fence-equal grant is flagged stale", staleAtFence.stale === 7n);
check("a rejected grant keeps the old generation", staleAtFence.generation === 7n);
const staleBelow = advanceMotionLease(afterRelease, response(true, 5));
check("a grant BELOW the fence is rejected", staleBelow.granted === false && staleBelow.stale === 5n);

// A denial after release is terminal.
const afterDenied = advanceMotionLease(afterRelease, response(false, 0, 3));
check("a denial sets terminal denied", afterDenied.denied === true);
check("a denial does not grant", afterDenied.granted === false);
check("a denial records the reason", afterDenied.denialReason === 3);

// An unrelated message kind leaves the state unchanged.
const unchanged = advanceMotionLease(afterGrant, { kind: "Pong", message: {} });
check("an unrelated message is a no-op", unchanged === afterGrant);

if (failures > 0) {
  console.error(`${failures} failure(s)`);
  process.exit(1);
}
console.log("all motion-lease transition checks passed");
