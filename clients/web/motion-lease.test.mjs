// The reliable-stream motion-lease transitions that main.js's session reader
// runs in production: release acknowledgement → motion response → generation
// installation, with the host-acknowledged fence, terminal denial, and u64
// generation wraparound enforced off the wire.

import { initialMotionLease, advanceMotionLease, isFreshGeneration } from "./motion-lease.js";
import {
  FRAME_REJECTION_UPLINK_IDLE,
  motionReadoutState,
  updateNeedsArm,
} from "./control-enactment.js";
import { CONTROL_ACTION } from "./wire.js";

let failures = 0;
function check(name, ok) {
  if (ok) {
    console.log(`ok   - ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL - ${name}`);
  }
}

const released = (generation) => ({ kind: "LeaseReleased", message: { released: true, generation } });
const response = (granted, generation, reason) => ({
  kind: "LeaseResponse",
  message: { granted, generation, reason },
});

// Steady authority held on generation 7.
const held = { ...initialMotionLease(), granted: true, generation: 7n };

// A release fences the HOST'S acknowledged generation (9), not the locally held
// one (7) — the host may fence ahead of what the client last saw.
const afterRelease = advanceMotionLease(held, released(9n));
check("release drops the grant", afterRelease.granted === false);
check("release fences the host's acknowledged generation", afterRelease.fence === 9n);

// A grant newer than the LOCAL gen but NOT the host fence (8 <= 9) is rejected.
const belowHostFence = advanceMotionLease(afterRelease, response(true, 8n));
check("a grant below the host fence is rejected", belowHostFence.granted === false);
check("the sub-fence grant is flagged stale", belowHostFence.stale === 8n);

// A regrant strictly newer than the host fence (10 > 9) installs and re-grants.
const afterGrant = advanceMotionLease(afterRelease, response(true, 10n));
check("a grant past the host fence is accepted", afterGrant.granted === true);
check("the fresh generation installs", afterGrant.generation === 10n);
check("a granted response is not denied", afterGrant.denied === false);

// A grant AT the fence is rejected (not strictly newer).
check(
  "a grant at the fence is rejected",
  advanceMotionLease(afterRelease, response(true, 9n)).granted === false,
);

// u64 generation ordering wraps: MAX → 0 is an advance; backward is not.
const U64_MAX = (1n << 64n) - 1n;
check("normal advance is fresh", isFreshGeneration(8n, 7n) === true);
check("same generation is not fresh", isFreshGeneration(7n, 7n) === false);
check("older generation is not fresh", isFreshGeneration(5n, 7n) === false);
check("u64 wrap MAX->0 is fresh", isFreshGeneration(0n, U64_MAX) === true);
check("backward across the wrap is not fresh", isFreshGeneration(U64_MAX, 0n) === false);
// A release fencing u64::MAX, then a wrapped grant of 0, is accepted.
const wrapped = advanceMotionLease(advanceMotionLease(held, released(U64_MAX)), response(true, 0n));
check("a wrapped regrant past a MAX fence installs", wrapped.granted === true && wrapped.generation === 0n);

// A denial is TERMINAL: a later grant must NOT restore authority.
const afterDenied = advanceMotionLease(afterRelease, response(false, 0n, 3));
check("a denial sets terminal denied", afterDenied.denied === true && afterDenied.granted === false);
check("a denial records the reason", afterDenied.denialReason === 3);
const grantAfterDenial = advanceMotionLease(afterDenied, response(true, 100n));
check("a grant after denial does NOT restore authority", grantAfterDenial.granted === false);
check("denial persists through a later grant", grantAfterDenial.denied === true);

// An unrelated message kind leaves the state unchanged.
const unchanged = advanceMotionLease(afterGrant, { kind: "Pong", message: {} });
check("an unrelated message is a no-op", unchanged === afterGrant);

// Lease and recovery are authority truth, not adapter enactment truth. An
// idle-uplink rejection latches the readout until an accepted ARM result.
const livePlan = { motion: { roll: 0, pitch: 0, throttle: 0, yaw: 0 } };
let needsArm = updateNeedsArm(false, {
  kind: "FrameRejected",
  reason: FRAME_REJECTION_UPLINK_IDLE,
});
check("uplink idle latches needs-arm", needsArm === true);
check(
  "lease plus recovery cannot overstate enactment",
  motionReadoutState(livePlan, { motionRecovered: true, needsArm }) === "needs arm",
);
needsArm = updateNeedsArm(needsArm, {
  kind: "ControlActionResult",
  action: CONTROL_ACTION.arm,
  accepted: false,
});
check("a refused arm cannot clear needs-arm", needsArm === true);
needsArm = updateNeedsArm(needsArm, {
  kind: "ControlActionResult",
  action: CONTROL_ACTION.modeRequest,
  accepted: true,
});
check("an unrelated result cannot clear needs-arm", needsArm === true);
needsArm = updateNeedsArm(needsArm, {
  kind: "ControlActionResult",
  action: CONTROL_ACTION.arm,
  accepted: true,
});
check("accepted arm restores enacted state", needsArm === false);
check(
  "the readout returns to streaming on the positive signal",
  motionReadoutState(livePlan, { motionRecovered: true, needsArm }) === "streaming",
);
needsArm = updateNeedsArm(needsArm, {
  kind: "ControlActionResult",
  action: CONTROL_ACTION.disarm,
  accepted: true,
});
check("accepted disarm retires enacted state", needsArm === true);

if (failures > 0) {
  console.error(`${failures} failure(s)`);
  process.exit(1);
}
console.log("all motion-lease transition checks passed");
