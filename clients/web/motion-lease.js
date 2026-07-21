// Pure motion-lease authority transitions for the reliable session stream.
//
// After a profile handover the runtime releases and reacquires the motion
// lease. The host fences the released generation; the client must not resume
// on a stale one. These transitions enforce that fence and the terminal denial,
// so main.js's reader stays thin and the policy is unit-tested off the wire.
//
// The canonical authority state is `{ granted, generation, fence, denied }`
// (generation and fence are BigInt). A `LeaseReleased` fences the generation we
// held; a `LeaseResponse` grant is accepted ONLY when its generation strictly
// exceeds that fence; a denial is terminal.

/** The authority state before any motion lease has been granted. */
export function initialMotionLease() {
  return { granted: false, generation: 0n, fence: 0n, denied: false };
}

const U64 = 1n << 64n;
const U64_HALF = 1n << 63n;

/** Whether `candidate` is a strictly-newer generation than `fence` under u64
 *  modular ordering, so a wrap (`u64::MAX → 0`) still reads as an advance. Only
 *  the forward half-window is accepted; the exact half and any backward delta
 *  are rejected — fail closed, never resume on an ambiguous or older generation
 *  (generations are u64 and wrap; a raw `>` would mis-order a wrap). */
export function isFreshGeneration(candidate, fence) {
  const delta = (((candidate - fence) % U64) + U64) % U64;
  return delta > 0n && delta < U64_HALF;
}

/** Applies a decoded reliable-stream message — already filtered to this vehicle
 *  and the motion scope — to the motion-lease authority, returning the next
 *  state. Unrelated message kinds return the state unchanged.
 *
 *  On a `LeaseResponse` grant whose generation is NOT strictly newer than the
 *  fence, the grant is rejected (stays ungranted) and the result carries a
 *  `stale` field with the offending generation. A denial sets `denied` (with
 *  `denialReason`) and is terminal. */
export function advanceMotionLease(motion, decoded) {
  // A denial is TERMINAL for the transport session: once denied, every further
  // response is ignored (a later grant must NOT restore authority) until a new
  // session resets the authority via initialMotionLease / the reconnect path.
  if (motion.denied) {
    return motion;
  }
  if (decoded.kind === "LeaseReleased") {
    // Fence the host's ACKNOWLEDGED generation — the protocol's current fencing
    // generation — not our locally-held one; the next grant must exceed it.
    const fence = BigInt(decoded.message.generation ?? motion.generation);
    return { granted: false, generation: motion.generation, fence, denied: false };
  }
  if (decoded.kind === "LeaseResponse") {
    const message = decoded.message;
    if (!message.granted) {
      return {
        granted: false,
        generation: motion.generation,
        fence: motion.fence,
        denied: true,
        denialReason: message.reason ?? null,
      };
    }
    const generation = BigInt(message.generation ?? 0);
    if (!isFreshGeneration(generation, motion.fence)) {
      // A grant not strictly newer than the fence is stale/replayed (or an
      // ambiguous wrap): never resume on it.
      return {
        granted: false,
        generation: motion.generation,
        fence: motion.fence,
        denied: false,
        stale: generation,
      };
    }
    return { granted: true, generation, fence: motion.fence, denied: false };
  }
  return motion;
}

/** Whether a `LinkLossCleared` message confirms the recovery the client is
 *  waiting on: the same vehicle, the motion scope, and the CURRENT fresh
 *  generation. A notice for another vehicle/scope, or a stale generation
 *  (e.g. the pre-handover one), proves nothing and must NOT resume control. */
export function isMotionRecoveryConfirmation(message, vehicleId, motionScope, currentGeneration) {
  return (
    message.vehicleId === vehicleId &&
    message.scope === motionScope &&
    message.generation === currentGeneration
  );
}
