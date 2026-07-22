// The input-loss gate for the control loop.
//
// Focus loss must be LATCHED by the blur event itself, not polled by the
// 30 Hz loop: a blur/refocus that fits entirely between two ticks — or a
// blur that fires while the loop is parked on an await — is invisible to a
// poll, so the loop would keep publishing on the same lease with freshly
// cleared arm-edge state (a still-held arm button would read as a new
// rising edge). Once latched, no frame may be sent until an explicit connect
// or same-session resume begins a fresh authority attempt.

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
    /** An explicit connect or resume re-arms the gate before its first await. */
    reset() {
      latched = false;
    },
  };
}
