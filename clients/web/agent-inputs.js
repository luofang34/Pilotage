// What the agent module reads from this client: the advertisement of the
// motion scope, and the telemetry that the instruments already admitted. The
// agent has no data path of its own (ADR-0042).

import { CONTROL_ACTION } from "./wire.js";
import { actionAdvertised } from "./typed-command.js";

// Valid-flag bits of the avionics snapshot.
const VALID_ATTITUDE = 1;
const VALID_POSITION = 4;
const VALID_VELOCITY = 8;

/** What the motion scope offers the agent now, or the reason it cannot fly. */
export function agentOffer(state, vehicleId, velocityCapability) {
  if (!state.connected) return { reason: "there is no session" };
  if (!velocityCapability || !(velocityCapability.maxLinear > 0)) {
    return { reason: `${state.motionScope} advertises no velocity intent` };
  }
  const offers = (action) =>
    actionAdvertised(state.advertisedScopes, vehicleId, state.motionScope, action);
  if (!offers(CONTROL_ACTION.arm)) {
    return { reason: `${state.motionScope} advertises no arm action` };
  }
  return {
    maxLinearMps: velocityCapability.maxLinear,
    disarmOffered: offers(CONTROL_ACTION.disarm),
  };
}

/** The operational estimate for the agent, or null when the snapshot does not
 *  have a valid attitude, position and velocity. A missing group is not a
 *  zero: a zero position is the launch point, and a zero yaw is north. */
export function agentTelemetry(snapshot, fcView) {
  const needed = VALID_ATTITUDE | VALID_POSITION | VALID_VELOCITY;
  if (!snapshot || (snapshot.validFlags & needed) !== needed) return null;
  const quat = snapshot.attitude?.quat;
  const kinematics = snapshot.kinematics;
  if (!quat || !kinematics) return null;
  return {
    posNed: Float64Array.from(kinematics.posNed),
    velNed: Float64Array.from(kinematics.velNed),
    quatWxyz: Float64Array.of(quat.w, quat.x, quat.y, quat.z),
    armState: fcView && !fcView.stale ? fcView.armState : 0,
  };
}
