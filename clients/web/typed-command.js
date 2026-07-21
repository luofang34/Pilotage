// Typed-command construction (CTRL-01): scales the control runtime's
// normalized plan onto the vehicle's ADVERTISED intent envelope, so full
// stick commands exactly what the vehicle negotiated — and nothing sends
// without a matching advertisement (fail closed). Pure functions, unit
// tested off the DOM.

// Wire IntentFamily values (capability.proto).
export const INTENT_FAMILY_VELOCITY = 1;
export const INTENT_FAMILY_GIMBAL_RATE = 5;

/** The advertised intent capability of `family` for `scope`, or null. */
export function intentCapabilityFor(advertisedScopes, scope, family) {
  for (const descriptor of advertisedScopes ?? []) {
    if (descriptor.scope !== scope) continue;
    for (const intent of descriptor.intents) {
      if (intent.family === family) return intent;
    }
  }
  return null;
}

/**
 * Builds the typed velocity intent (m/s, rad/s) from the plan's normalized
 * motion demands. Rover mode drives surge/turn only; the flight modes map
 * pitch=forward, roll=right, +throttle=climb (body-FRD +z is down). Returns
 * null without a velocity advertisement — the caller must not send.
 */
export function buildVelocityIntent(motion, mode, capability) {
  if (!capability) return null;
  const maxVertical = capability.maxVertical || capability.maxLinear;
  return mode === "rover"
    ? {
        vx: motion.throttle * capability.maxLinear,
        vy: 0,
        vz: 0,
        yawRate: motion.yaw * capability.maxAngular,
      }
    : {
        vx: motion.pitch * capability.maxLinear,
        vy: motion.roll * capability.maxLinear,
        vz: -motion.throttle * maxVertical,
        yawRate: motion.yaw * capability.maxAngular,
      };
}

/**
 * Builds the typed gimbal-rate intent (rad/s) from the plan's normalized
 * LOS rates. Returns null without a gimbal-rate advertisement.
 */
export function buildGimbalRateIntent(gimbal, capability) {
  if (!capability) return null;
  return {
    pitchRate: gimbal.pitch * capability.maxAngular,
    yawRate: gimbal.yaw * capability.maxAngular,
  };
}
