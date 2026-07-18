// The input-loss gate for the control loop (CTRL-04, #147).
//
// Focus loss must be LATCHED by the blur event itself, not polled by the
// 30 Hz loop: a blur/refocus that fits entirely between two ticks — or a
// blur that fires while the loop is parked on an await — is invisible to a
// poll, so the loop would keep publishing on the same lease with freshly
// cleared arm-edge state (a still-held arm button would read as a new
// rising edge). Once latched, no frame may be sent under that generation;
// only an explicit new connect (a fresh lease) resets the gate.

/**
 * @param {{ isFocused: () => boolean }} deps live focus probe, kept as a
 *   belt-and-braces check alongside the latch
 */
export function createControlGate({ isFocused }) {
  let latched = false;
  return {
    /** Called synchronously from the blur event: latches input loss. */
    latchInputLoss() {
      latched = true;
    },
    /** Whether input loss has been latched since the last reset. */
    isLatched() {
      return latched;
    },
    /**
     * Whether the loop may publish a control frame right now. False once
     * latched — a later refocus does NOT clear it — and false while
     * actually unfocused even if no blur event was delivered.
     */
    mayPublish() {
      return !latched && isFocused();
    },
    /** A fresh explicit connect (new lease/generation) re-arms the gate. */
    reset() {
      latched = false;
    },
  };
}
