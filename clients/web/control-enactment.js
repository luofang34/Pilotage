import { CONTROL_ACTION } from "./wire.js";

/** Wire reason assigned to an admitted frame refused by an idle uplink. */
export const FRAME_REJECTION_UPLINK_IDLE = 18;

/** Advances the client latch that distinguishes authority from enactment. */
export function updateNeedsArm(current, event) {
  if (event.kind === "FrameRejected" && event.reason === FRAME_REJECTION_UPLINK_IDLE) {
    return true;
  }
  if (
    event.kind === "ControlActionResult" &&
    event.action === CONTROL_ACTION.arm &&
    event.accepted
  ) {
    return false;
  }
  if (
    event.kind === "ControlActionResult" &&
    event.action === CONTROL_ACTION.disarm &&
    event.accepted
  ) {
    return true;
  }
  return current;
}

/** Labels whether motion is gated, recovering, waiting for arm, or enacted. */
export function motionReadoutState(plan, { motionRecovered, needsArm }) {
  if (needsArm) return "needs arm";
  if (!plan.motion) return "gated";
  return motionRecovered ? "streaming" : "recovering";
}
