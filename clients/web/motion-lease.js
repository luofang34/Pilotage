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

/** Applies a decoded reliable-stream message — already filtered to this vehicle
 *  and the motion scope — to the motion-lease authority, returning the next
 *  state. Unrelated message kinds return the state unchanged.
 *
 *  On a `LeaseResponse` grant whose generation is NOT strictly newer than the
 *  fence, the grant is rejected (stays ungranted) and the result carries a
 *  `stale` field with the offending generation. A denial sets `denied` (with
 *  `denialReason`) and is terminal. */
export function advanceMotionLease(motion, decoded) {
  if (decoded.kind === "LeaseReleased") {
    // Fence the generation we held; the next grant must strictly exceed it.
    return {
      granted: false,
      generation: motion.generation,
      fence: motion.generation,
      denied: motion.denied,
    };
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
    if (generation <= motion.fence) {
      // A grant at or below the fence is stale/replayed: never resume on it.
      return {
        granted: false,
        generation: motion.generation,
        fence: motion.fence,
        denied: motion.denied,
        stale: generation,
      };
    }
    return { granted: true, generation, fence: motion.fence, denied: false };
  }
  return motion;
}
