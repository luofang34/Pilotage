// WebTransport lifecycle ownership for the demo viewer. A token is an object,
// not only its wrapping generation, so a delayed callback cannot become active
// again after the numeric generation wraps.

import { streamCancellationReason } from "./stream-cancellation.js";

function discardTeardownRejection(pending) {
  if (pending && typeof pending.catch === "function") pending.catch(() => {});
}

function cancelReader(reader, reason) {
  try {
    discardTeardownRejection(reader.cancel(reason));
  } catch {
    // Teardown is already fail-closed because the token was invalidated first.
  }
}

function abortWriter(writer) {
  try {
    discardTeardownRejection(writer.abort());
  } catch {
    // Teardown is already fail-closed because the token was invalidated first.
  }
}

function closeTransport(transport) {
  try {
    transport.close();
  } catch {
    // An already-failed transport still loses authority through its token.
  }
}

function disposeSession(session, shouldCloseTransport, reason) {
  for (const reader of session.readers) cancelReader(reader, reason);
  for (const writer of session.writers) abortWriter(writer);
  session.readers.clear();
  session.writers.clear();
  if (shouldCloseTransport) closeTransport(session.token.transport);
}

export class TransportSessionLifecycle {
  constructor() {
    this.generation = 0;
    this.active = null;
  }

  begin(transport) {
    if (!transport || typeof transport.close !== "function") {
      throw new TypeError("transport must provide close()");
    }
    const previous = this.active;
    this.generation = (this.generation + 1) >>> 0;
    const token = Object.freeze({ generation: this.generation, transport });
    this.active = { token, readers: new Set(), writers: new Set() };
    if (previous) {
      disposeSession(
        previous,
        true,
        streamCancellationReason("session-replaced"),
      );
    }
    return token;
  }

  isActive(token) {
    return this.active?.token === token;
  }

  currentToken() {
    return this.active?.token ?? null;
  }

  runIfActive(token, action) {
    if (!this.isActive(token)) return false;
    action();
    return true;
  }

  trackReader(token, reader, inactiveReason = streamCancellationReason("session-inactive")) {
    if (!reader || typeof reader.cancel !== "function") {
      throw new TypeError("reader must provide cancel()");
    }
    if (!this.isActive(token)) {
      cancelReader(reader, inactiveReason);
      return false;
    }
    this.active.readers.add(reader);
    return true;
  }

  untrackReader(token, reader) {
    if (this.isActive(token)) this.active.readers.delete(reader);
  }

  trackWriter(token, writer) {
    if (!writer || typeof writer.abort !== "function") {
      throw new TypeError("writer must provide abort()");
    }
    if (!this.isActive(token)) {
      abortWriter(writer);
      return false;
    }
    this.active.writers.add(writer);
    return true;
  }

  untrackWriter(token, writer) {
    if (this.isActive(token)) this.active.writers.delete(writer);
  }

  close(token) {
    if (!this.isActive(token)) return false;
    const session = this.active;
    this.active = null;
    disposeSession(session, true, streamCancellationReason("session-closed"));
    return true;
  }

  retire(token) {
    if (!this.isActive(token)) return false;
    const session = this.active;
    this.active = null;
    disposeSession(session, false, streamCancellationReason("session-retired"));
    return true;
  }
}
