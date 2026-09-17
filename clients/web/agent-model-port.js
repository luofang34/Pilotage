// The model port of the agent module (ADR-0042), as a browser reaches it.
//
// A model is a separate process and a browser cannot start one. A model
// gateway holds the adapter process and gives the same exchange over HTTP: one
// request line in, one reply line out. This module moves those lines and does
// not read them. A failure becomes a fault line, so the shared agent core
// classifies each result in one place.

/** The reply line for a fault that the gateway itself did not write. */
function faultLine(detail) {
  return JSON.stringify({ error: detail });
}

/**
 * Creates the connection to one model gateway.
 *
 * `fetchImpl` and `timeoutMs` are parameters so that a test can drive the
 * port with no network and no wait.
 */
export function createModelPort({ baseUrl, fetchImpl = globalThis.fetch, timeoutMs = 30_000 }) {
  const root = String(baseUrl).replace(/\/+$/, "");

  async function exchange(path, init) {
    const abort = new AbortController();
    const timer = setTimeout(() => abort.abort(), timeoutMs);
    try {
      const response = await fetchImpl(`${root}${path}`, { ...init, signal: abort.signal });
      const text = (await response.text()).trim();
      if (!response.ok) {
        return faultLine(`the model gateway answered ${response.status}: ${text.slice(0, 200)}`);
      }
      return text;
    } catch (error) {
      const reason = abort.signal.aborted ? `no answer in ${timeoutMs} ms` : String(error);
      return faultLine(`the model gateway is not reachable at ${root}: ${reason}`);
    } finally {
      clearTimeout(timer);
    }
  }

  return {
    baseUrl: root,
    /** The adapter declaration line, or a fault line. */
    declaration: () => exchange("/v1/declaration", { method: "GET" }),
    /** One request line in, one reply line out. */
    ask: (requestLine) =>
      exchange("/v1/directive", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: requestLine,
      }),
  };
}

/** The gateway address: the `agent` URL parameter, or port 8098 on the host
 *  that serves the page. */
export function gatewayAddress(location) {
  const named = new URLSearchParams(location.search).get("agent");
  if (named) return named;
  return `${location.protocol}//${location.hostname}:8098`;
}
