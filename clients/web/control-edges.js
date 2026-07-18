// Pure arm/disarm edge detection for the demo viewer's control loop.
//
// Arm/disarm are one-shot RISING edges: a command is sent only when an input
// that was NOT pressed last tick is pressed now. The subtlety this module
// exists to make testable is priming — seeding the "previously pressed" set to
// whatever is already held when control (re)starts. Without it, a button held
// down across a reconnect (or across control being re-enabled) reads as a fresh
// press and fires a spurious arm/disarm. Clearing the set on focus loss serves
// the same end: a key still "held" only because its keyup was swallowed by the
// blur must not edge when the window returns.

/// The set of arm/disarm inputs currently held, from raw button/key booleans.
export function pressedArmInputs({ padArm, padDisarm, keyArm, keyDisarm }) {
  const pressed = new Set();
  if (padArm) pressed.add("pad-arm");
  if (padDisarm) pressed.add("pad-disarm");
  if (keyArm) pressed.add("key-arm");
  if (keyDisarm) pressed.add("key-disarm");
  return pressed;
}

/// The inputs in `pressedNow` that were NOT in `prevPressed` — the rising edges.
/// A still-held input (present in both) yields nothing, which is exactly why
/// priming `prevPressed` to the held set suppresses a hold-across-reconnect edge.
export function risingArmEdges(pressedNow, prevPressed) {
  const edges = [];
  for (const which of pressedNow) {
    if (!prevPressed.has(which)) edges.push(which);
  }
  return edges;
}
