// Session discovery for launcher-driven viewers (#182).
//
// Every launcher run mints a fresh certificate and pins it in the opened
// URL, so a tab left over from an earlier session retries forever against
// a host whose certificate no longer matches — and a refresh of that tab
// cannot converge, because its URL still carries the dead hash. The
// launcher therefore serves the CURRENT session's connect parameters next
// to the page (`session.json`), and the viewer re-reads them after a
// failed connect so any tab converges on the live session.
//
// Pure DOM-free helpers, unit tested off the page; main.js wires them to
// the real inputs, fetch, and document.

/** Validates a fetched session manifest. Returns the normalized
 *  `{ host, port, certHash }` (port as the input-box string), or null
 *  unless every field is present and usable — a partial manifest must
 *  never half-update the connect inputs. */
export function validSessionConfig(config) {
  if (!config || typeof config !== "object") return null;
  const { host, port, certHash } = config;
  if (typeof host !== "string" || host.trim().length === 0) return null;
  if (!Number.isInteger(port) || port <= 0 || port > 65535) return null;
  if (typeof certHash !== "string" || !/^[0-9a-fA-F]{64}$/.test(certHash)) return null;
  return { host: host.trim(), port: String(port), certHash };
}

/** Applies a validated config to the connect inputs, reporting whether
 *  anything actually changed (the caller logs only real updates). */
export function applySessionConfig(els, config) {
  const changed =
    els.host.value !== config.host ||
    els.port.value !== config.port ||
    els.certHash.value !== config.certHash;
  els.host.value = config.host;
  els.port.value = config.port;
  els.certHash.value = config.certHash;
  return changed;
}

/** Whether a failed manifest re-read proves the LAUNCHER session is over,
 *  rather than the manifest merely never having existed (a viewer served
 *  without the launcher has none, and silence is correct there). The
 *  launcher deletes `session.json` when its session ends and its static
 *  server dies with it, so in a launcher context — the URL was
 *  launcher-pinned, or a manifest WAS served earlier — a manifest that is
 *  now absent (`"missing"`) or unfetchable (`"unreachable"`) means no
 *  reconnect can succeed until a NEW session writes a fresh manifest. */
export function launcherSessionOver(launcherContext, outcome) {
  return launcherContext && (outcome === "missing" || outcome === "unreachable");
}

/** Runs `begin` once `doc` is visible — immediately when it already is,
 *  otherwise on the first visibilitychange that lands visible. A hidden
 *  launcher-opened tab must not autoconnect: its throttled timers starve
 *  the 30 Hz control loop into watchdog churn, and an unfocused connect
 *  silently skips motion authority. */
export function whenVisible(doc, begin) {
  if (doc.visibilityState === "visible") {
    begin();
    return;
  }
  const onChange = () => {
    if (doc.visibilityState !== "visible") return;
    doc.removeEventListener("visibilitychange", onChange);
    begin();
  };
  doc.addEventListener("visibilitychange", onChange);
}
