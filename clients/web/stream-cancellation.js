/** A machine-readable reason passed to `ReadableStream` cancellation. */
export class StreamCancellationReason extends Error {
  constructor(kind, detail) {
    super(detail ? `${kind}: ${detail}` : kind);
    this.name = "StreamCancellationReason";
    this.kind = kind;
  }
}

/** Builds a typed cancellation reason without erasing the triggering error. */
export function streamCancellationReason(kind, cause = null) {
  const detail = cause instanceof Error ? cause.message : cause === null ? "" : String(cause);
  const reason = new StreamCancellationReason(kind, detail);
  if (cause !== null) reason.cause = cause;
  return reason;
}
