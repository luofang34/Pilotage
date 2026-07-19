// Gimbal quasimode input (GIM-03, #167): LT held redirects the right stick
// from flight to `vehicle.gimbal` LOS-rate demands, and while it is held the
// right stick and LT read NEUTRAL to every flight scheme (cruise's LT-descend
// included) — the inhibition is a masked pad view, so no scheme can consume a
// captured input by accident. R3 (right-stick click) recenters the gimbal.
//
// Side-effect free by construction so the quasimode's safety properties —
// capture masking, entry/exit neutralization, one-shot recenter — are
// testable without a browser (gimbal-input.test.mjs).

export const PAD_GIMBAL_MODIFIER = 6; // L2/LT (Standard Gamepad analog trigger), owner decision #167.
export const PAD_GIMBAL_RESET = 11; // R3 (right-stick click): recenter the gimbal.

/** Deadzone + cubic-expo (50%) stick shaping shared by the flight and
 *  gimbal reads: fine authority near center, full range at the ends —
 *  half of the DJI feel; the uplink's slew limit is the other. */
export function stickShaper(profile) {
  const clamp = (v) => Math.max(-1, Math.min(1, v));
  const dz = profile?.deadzone ?? 0.1;
  const expo = (v) => 0.35 * v * v * v + 0.65 * v;
  return (v) => expo(clamp(Math.abs(v) < dz ? 0 : v));
}

/** Whether the gimbal quasimode is engaged: LT held on a Standard
 *  Gamepad (analog past half travel, or reported pressed). Non-standard
 *  layouts (EdgeTX radios) have no LT and never engage it. */
export function gimbalModifierHeld(pad, profile) {
  if (!pad || !profile?.standard) return false;
  const lt = pad.buttons?.[PAD_GIMBAL_MODIFIER];
  return !!lt && ((lt.value ?? 0) > 0.5 || lt.pressed === true);
}

/** Right-stick gimbal LOS rates under the quasimode: pitch + = camera
 *  up (stick up), yaw + = camera right. Same shaping as flight sticks. */
export function gimbalAxesFromGamepad(pad, profile) {
  const rawAt = (i) => (i >= 0 && i < pad.axes.length ? pad.axes[i] : 0);
  const shaped = stickShaper(profile);
  // Standard Gamepad: right stick X = axis 2, Y = axis 3 (up = negative).
  return { pitch: shaped(-rawAt(3)), yaw: shaped(rawAt(2)) };
}

/** The pad as flight sees it while the quasimode is engaged: the right
 *  stick (axes 2/3) and LT read neutral; everything else passes through
 *  untouched (RT climb, left stick, arm/disarm buttons). */
export function gimbalMaskedView(pad) {
  return {
    id: pad.id,
    connected: pad.connected,
    axes: Array.from(pad.axes, (v, i) => (i === 2 || i === 3 ? 0 : v)),
    buttons: Array.from(pad.buttons ?? [], (b, i) =>
      i === PAD_GIMBAL_MODIFIER ? { pressed: false, touched: false, value: 0 } : b,
    ),
  };
}

/** Decides one tick's ACTIVE gimbal demand. Returns null when there is
 *  no active demand — the caller still sends an idle zero-rate frame
 *  while the lease is held, because a continuous stream is the scope's
 *  liveness (a quiet holder trips the host's holder-silence watchdog,
 *  whose link-loss policy is per-vehicle). Otherwise
 *  `{ rates, recenter, streaming }`, where an exit from the quasimode
 *  yields exactly one trailing neutral frame; callers thread
 *  `streaming` back in on the next tick. */
export function gimbalFramePlan({ held, resetEdge, streaming, rates }) {
  if (!held && !resetEdge && !streaming) return null;
  return {
    rates: held ? rates : { pitch: 0, yaw: 0 },
    recenter: resetEdge,
    streaming: held,
  };
}
