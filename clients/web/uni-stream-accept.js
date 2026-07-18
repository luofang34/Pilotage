// The demo viewer's incoming-uni-stream accept loop, factored out so its
// lifecycle can be tested against real ReadableStreams.
//
// `WebTransport.incomingUnidirectionalStreams` is a SINGLE ReadableStream for
// the whole session. Two properties of the WHATWG Streams model drive this
// design:
//
//   1. An individual receive stream (one video frame) failing is independent of
//      the collection: it loses that one frame, and the loop keeps accepting
//      later streams.
//   2. The collection stream closing or erroring is TERMINAL for the session.
//      Once it stores an error, every future reader — including one obtained by
//      releasing the lock and calling getReader() again — is handed back the
//      identical stored error. Reacquiring a reader therefore cannot recover;
//      the only correct response is to surface one session failure and stop.
//
// So there is exactly one accept loop per WebTransport session, and a terminal
// collection end is reported once via `onCollectionTerminal`, never retried.

/**
 * Runs one accept loop over an incoming-uni-stream collection.
 *
 * @param incomingStreams the session's `incomingUnidirectionalStreams`
 * @param cb callbacks:
 *   - `isActive()` — whether the session is still current; the loop stops
 *     (without reporting a terminal) as soon as it returns false.
 *   - `handleStream(stream)` — drains one received stream; may reject, which
 *     loses only that frame and is reported through `onStreamFailure`.
 *   - `onStreamFailure(error)` — a single stream's drain rejected.
 *   - `onCollectionTerminal(errorOrNull)` — the collection closed (`null`) or
 *     errored (the stored error). Terminal for the session; fired at most once.
 *   - `trackReader(reader)` / `untrackReader(reader)` — optional session
 *     bookkeeping so teardown can cancel a blocked read; `trackReader` may
 *     return false to abort before the loop starts.
 */
export async function runIncomingStreamAcceptLoop(
  incomingStreams,
  { isActive, handleStream, onStreamFailure, onCollectionTerminal, trackReader, untrackReader },
) {
  if (!isActive()) return;
  const reader = incomingStreams.getReader();
  if (trackReader && !trackReader(reader)) {
    releaseReaderLock(reader);
    return;
  }
  try {
    for (;;) {
      const { value: stream, done } = await reader.read();
      if (!isActive()) return;
      if (done) {
        // The collection closed gracefully — the session is ending. Terminal.
        onCollectionTerminal(null);
        return;
      }
      // An individual stream's drain rejecting loses only that frame; keep
      // accepting. Never let it reach the collection-level terminal path.
      Promise.resolve(handleStream(stream)).catch((error) => {
        if (isActive()) onStreamFailure(error);
      });
    }
  } catch (error) {
    // The collection stream itself errored: terminal for the session. Do NOT
    // reacquire a reader — it would return this same stored error.
    if (isActive()) onCollectionTerminal(error);
  } finally {
    if (untrackReader) untrackReader(reader);
    releaseReaderLock(reader);
  }
}

function releaseReaderLock(reader) {
  try {
    reader.releaseLock();
  } catch {
    // Lock already released, or the stream errored with a pending read; there
    // is nothing to release and no recovery to attempt.
  }
}
