/** Application close code shared with the Pilotage host. */
export const ADMISSION_REJECTED_CLOSE_CODE = 0x5049_0001;

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
