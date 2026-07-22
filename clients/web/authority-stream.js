// Incremental decoding for the long-lived authority-events stream. Its bytes
// arrive fragmented and the stream never closes during a session, so envelopes
// must be decoded and dispatched AS THEY COMPLETE — never buffered until close,
// which would strand a recovery acknowledgement forever.

/** Drains every COMPLETE length-delimited envelope from `buf`, dispatching each
 *  through `onEnvelope`, and returns the leftover (incomplete) tail to carry
 *  into the next chunk. `decode` returns `{ consumed, kind, message }` for a
 *  complete envelope or a falsy value when `buf` holds only a partial one. */
export function drainAuthorityEnvelopes(buf, decode, onEnvelope) {
  for (;;) {
    const decoded = decode(buf);
    if (!decoded) return buf;
    buf = buf.subarray(decoded.consumed);
    onEnvelope(decoded);
  }
}
