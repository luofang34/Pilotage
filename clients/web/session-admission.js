/** Application close code shared with the Pilotage host. */
export const ADMISSION_REJECTED_CLOSE_CODE = 0x5049_0001;

/** URL parameters can select a target, but cannot supply its credential. */
export function applySessionTargetParams(els, search) {
  const params = new URLSearchParams(search);
  if (params.has("host")) els.host.value = params.get("host");
  if (params.has("port")) els.port.value = params.get("port");
  if (params.has("cert")) els.certHash.value = params.get("cert");
}

/** Builds one session URL from a DNS name, IPv4 address, or IPv6 address. */
export function pilotageSessionUrl(host, port) {
  const authority = host.includes(":") && !host.startsWith("[") ? `[${host}]` : host;
  const url = new URL(`https://${authority}:${port}/pilotage`);
  if (url.username || url.password || url.search || url.hash || url.pathname !== "/pilotage") {
    throw new TypeError("host must be a DNS name or IP address");
  }
  return url.href;
}

/** Parses a join code without including its value in an error. */
export function joinCodeBytes(value) {
  const code = value.trim();
  if (code === "") return new Uint8Array(0);
  if (!/^[0-9a-fA-F]{64}$/.test(code)) {
    throw new TypeError("join code must contain exactly 64 hexadecimal characters");
  }
  return Uint8Array.from(code.match(/../g), (pair) => Number.parseInt(pair, 16));
}

/** Keeps a credential bound to the address and certificate the user selected. */
export function createJoinCredentials() {
  let admittedTarget = null;
  let credential = new Uint8Array(0);
  return {
    forConnection(target, value, manual) {
      if (manual) {
        credential = joinCodeBytes(value);
        admittedTarget = target;
      } else if (credential.length && target !== admittedTarget) {
        throw new TypeError("host identity changed; check the host and press Connect");
      }
      return credential.slice();
    },
  };
}

/** Validates the target before binding a credential to the next connection. */
export function sessionConnectionParameters(els, credentials, manual, log) {
  const host = els.host.value.trim();
  const port = els.port.value.trim();
  const certHashHex = els.certHash.value.trim();
  if (!host || !port || !certHashHex) {
    log("host, port, and cert hash are all required");
    return null;
  }
  if (!/^[0-9a-fA-F]{64}$/.test(certHashHex)) {
    log("certificate hash must be exactly 64 hexadecimal characters (SHA-256)");
    return null;
  }
  const certHash = Uint8Array.from(certHashHex.match(/../g), (pair) => Number.parseInt(pair, 16));
  try {
    const url = pilotageSessionUrl(host, port);
    const joinToken = credentials.forConnection(
      `${url}#${certHashHex.toLowerCase()}`, els.joinCode.value, manual,
    );
    return { url, certHash, certHashHex, joinToken };
  } catch (error) {
    log(error.message);
    return null;
  }
}

/// Classifies a connect failure as retryable or not. A construction failure
/// (a bad URL or certificate hash) and an explicit host rejection
/// (authentication, protocol, or lease policy) cannot succeed on retry — they
/// need the user to act — so they stop auto-recovery. Anything else is treated
/// as a transient transport drop and retried. `failure.phase` is a structured
/// tag from the connect flow, never a matched error string.
export function classifyConnectFailure(failure) {
  if (!failure) return { retryable: true, kind: "unknown" };
  if (failure.phase === "construct") return { retryable: false, kind: "config" };
  if (failure.phase === "rejected") {
    return { retryable: false, kind: failure.kind || "rejected" };
  }
  return { retryable: true, kind: failure.kind || "transport" };
}

/** Uses the application code, never server-supplied text, to stop retries. */
export function transportCloseFailure(close) {
  if (close?.bootstrapRejected === true) return { phase: "rejected", kind: "bootstrap" };
  if (
    close?.closeCode === ADMISSION_REJECTED_CLOSE_CODE ||
    close?.streamErrorCode === ADMISSION_REJECTED_CLOSE_CODE
  ) {
    return { phase: "rejected", kind: "admission" };
  }
  return { phase: "transport" };
}

/** A stream EOF can arrive before the session's application close code. */
export async function failureAfterClose(
  closed,
  fallback,
  schedule = (callback) => setTimeout(callback, 1000),
  cancel = clearTimeout,
) {
  if (fallback.phase === "rejected") return fallback;
  let timer;
  try {
    return await Promise.race([
      closed,
      new Promise((resolve) => {
        timer = schedule(() => resolve(fallback));
      }),
    ]);
  } finally {
    cancel(timer);
  }
}
