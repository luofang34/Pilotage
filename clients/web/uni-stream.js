// One long-lived uni stream's body reader, factored out of main.js so the
// kind-tag peel and the LIVE authority-envelope dispatch can be tested against
// real ReadableStreams (the core of `readOneUniStream`).
//
// Every incoming uni stream leads with a one-byte kind tag (0x01
// authority-events, 0x02/0x03 one video frame). The authority stream is
// long-lived and NEVER closes during a session, so its envelopes must be
// decoded and dispatched AS THEY COMPLETE — buffering to close would strand a
// recovery acknowledgement forever. A video stream is one frame in its whole
// body, rendered from the returned tail at close.

import { drainAuthorityEnvelopes } from "./authority-stream.js";
import { streamCancellationReason } from "./stream-cancellation.js";

/** Appends `incoming` after `existing`. */
function appendBytes(existing, incoming) {
  const out = new Uint8Array(existing.length + incoming.length);
  out.set(existing, 0);
  out.set(incoming, existing.length);
  return out;
}

/**
 * Reads one uni stream to close from `reader`: peels the leading one-byte kind
 * tag, and for `authorityKind` decodes and dispatches every COMPLETE envelope
 * live through `onAuthorityEnvelope`. Returns `{ kind, tail, aborted }` — the
 * caller renders a video body from `tail` at close (authority has already
 * dispatched incrementally). `shouldContinue()`, when supplied, aborts the read
 * on session teardown and reports `aborted: true` so the caller skips its
 * close-time work.
 *
 * @param reader a `ReadableStreamDefaultReader` over the stream's bytes
 * @param cb `{ authorityKind, decode, onAuthorityEnvelope, shouldContinue }`
 */
export async function readUniStream(
  reader,
  { authorityKind, decode, onAuthorityEnvelope, shouldContinue, onCancelFailure },
) {
  async function cancel(kind, cause = null) {
    const reason = streamCancellationReason(kind, cause);
    try {
      await reader.cancel(reason);
    } catch (error) {
      onCancelFailure?.(error, reason);
    }
  }

  let buf = new Uint8Array(0);
  let kind = null;
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (shouldContinue && !shouldContinue()) {
        await cancel("stream-abandoned");
        return { kind, tail: buf, aborted: true };
      }
      if (value) buf = appendBytes(buf, value);
      // The one-byte kind tag leads the stream; peel it once it arrives.
      if (kind === null && buf.length >= 1) {
        kind = buf[0];
        buf = buf.subarray(1);
      }
      // Authority is long-lived: dispatch every complete envelope live so a
      // recovery ack is acted on immediately, never buffered until a close that
      // never comes.
      if (kind === authorityKind) {
        buf = drainAuthorityEnvelopes(buf, decode, onAuthorityEnvelope);
      }
      if (done) break;
    }
  } catch (error) {
    await cancel("stream-read-failed", error);
    throw error;
  }
  return { kind, tail: buf, aborted: false };
}
