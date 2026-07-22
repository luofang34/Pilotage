import { runBootstrapReader } from "./bootstrap.js";
import { negotiateSessionAuthority } from "./connect-authority.js";
import { createReconnectController } from "./reconnect.js";
import {
  applySessionConfig,
  launcherSessionOver,
  validSessionConfig,
} from "./session-discovery.js";
import { decodeLengthDelimitedEnvelope } from "./wire.js";

const SUPERSEDED = { ok: true };

/** Builds WebTransport construction, lifetime ownership, and reconnect wiring. */
export function createSessionTransport({
  state,
  els,
  transportSessions,
  controlGate,
  releaseTracker,
  motionScope,
  log,
  surface,
  readout,
  bootstrap,
  control,
}) {
  const discovery = {
    launcherPinned: new URLSearchParams(window.location.search).get("autoconnect") === "1",
    manifestSeen: false,
    endReported: false,
  };

  function applyUrlParams() {
    const params = new URLSearchParams(window.location.search);
    if (params.has("host")) els.host.value = params.get("host");
    if (params.has("port")) els.port.value = params.get("port");
    if (params.has("cert")) els.certHash.value = params.get("cert");
  }

  async function refreshSessionConfig() {
    let response = null;
    let outcome;
    try {
      response = await fetch("./session.json", { cache: "no-cache" });
      outcome = response.ok ? "fetched" : "missing";
    } catch {
      outcome = "unreachable";
    }
    if (outcome === "fetched") {
      let fetched = null;
      try {
        fetched = await response.json();
      } catch {
        return;
      }
      const config = validSessionConfig(fetched);
      if (!config) return;
      discovery.manifestSeen = true;
      discovery.endReported = false;
      if (applySessionConfig(els, config)) {
        log(
          `session discovery: connect target updated to ${config.host}:${config.port} ` +
            `(cert ${config.certHash.slice(0, 16)}...)`,
        );
      }
      return;
    }
    const context = discovery.launcherPinned || discovery.manifestSeen;
    if (launcherSessionOver(context, outcome) && !discovery.endReported) {
      discovery.endReported = true;
      log(
        "the launcher session is over (its manifest is gone) — reconnect attempts " +
          "cannot succeed until a new session starts (cargo xtask sim); " +
          "auto-reconnect keeps watching for one",
      );
    }
  }

  function hexToBytes(hex) {
    const clean = hex.trim().toLowerCase();
    const out = new Uint8Array(clean.length / 2);
    for (let i = 0; i < out.length; i += 1) {
      out[i] = Number.parseInt(clean.substr(i * 2, 2), 16);
    }
    return out;
  }

  async function connect({ manual } = { manual: true }) {
    const host = els.host.value.trim();
    const port = els.port.value.trim();
    const certHashHex = els.certHash.value.trim();
    if (!host || !port || !certHashHex) {
      log("host, port, and cert hash are all required");
      return { ok: false, failure: { phase: "construct" } };
    }
    if (!/^[0-9a-fA-F]{64}$/.test(certHashHex)) {
      log("certificate hash must be exactly 64 hexadecimal characters (SHA-256)");
      return { ok: false, failure: { phase: "construct" } };
    }
    const url = `https://${host}:${port}/pilotage`;
    const certHash = hexToBytes(certHashHex);
    const session = await negotiateSessionAuthority({
      manual,
      gate: controlGate,
      releases: releaseTracker,
      openAndBootstrap: (leaseProbe) =>
        openTransportSession({ url, certHash, certHashHex, manual, leaseProbe }),
      startControl: ({ transport, token }) => control.startControlLoop(transport, token),
      controlUnavailable: ({ token }) => {
        transportSessions.runIfActive(token, () => {
          control.sendLeaseRelease(token);
          control.startSuspendedPressWatch(token);
        });
      },
      releaseLease: ({ token }) => {
        transportSessions.runIfActive(token, () => {
          control.sendLeaseRelease(token);
          surface.inputLostDuringConnect();
          log("input lost during connect — lease released immediately");
          control.startSuspendedPressWatch(token);
        });
      },
      telemetryOnly: (connectedSession, wasManual) => {
        if (wasManual) {
          surface.noControlLease(true);
          log("no control lease granted; viewer is telemetry/video only");
        } else {
          surface.noControlLease(false);
          log("reconnected (telemetry/video); press Connect to resume control");
        }
        if (connectedSession?.token !== undefined) {
          control.startSuspendedPressWatch(connectedSession.token);
        }
      },
    });
    if (!session.result.ok && session.result.failure?.phase !== "construct") {
      void refreshSessionConfig();
    }
    return session.result;
  }

  async function openTransportSession({ url, certHash, certHashHex, manual, leaseProbe }) {
    let transport;
    try {
      transport = new WebTransport(url, {
        serverCertificateHashes: [{ algorithm: "sha-256", value: certHash }],
      });
    } catch (error) {
      log(`WebTransport creation failed: ${error}`);
      return { completed: false, result: { ok: false, failure: { phase: "construct" } } };
    }
    const token = transportSessions.begin(transport);
    transportSessions.runIfActive(token, () => {
      state.transport = transport;
      state.sessionWriter = null;
      state.sessionId = 0;
      state.sequence = 0;
      state.lastFrameRejectionLogged = null;
      state.gimbalSequence = 0;
      state.controlShell?.beginSession();
      state.pendingReset = false;
      state.pendingFpvToggle = false;
      state.lastFcVerdictLogged = null;
      state.lastFcView = null;
      state.connected = false;
      state.controlCompletion = null;
      state.stopControlRun = null;
      state.resumePendingToken = null;
      state.resumeGimbalLease = false;
      els.resumeBtn.hidden = true;
      state.skippedVideoFrames = 0;
      readout.resetVideoDiagnostics();
      readout.retireSessionPresentation("connecting");
      const mode = manual ? "requesting control" : "auto-reconnect, telemetry/video only";
      log(`connecting to ${url} (${mode}) pinned to cert hash ${certHashHex.slice(0, 16)}...`);
    });

    transport.closed.then(
      () => handleTransportClosed(token, null),
      (error) => handleTransportClosed(token, error),
    );

    try {
      await transport.ready;
      if (!transportSessions.isActive(token)) return { completed: false, result: SUPERSEDED };
      log("WebTransport session ready");
      const bidi = await transport.createBidirectionalStream();
      if (!transportSessions.isActive(token)) return { completed: false, result: SUPERSEDED };
      const writer = bidi.writable.getWriter();
      const reader = bidi.readable.getReader();
      if (!transportSessions.trackWriter(token, writer)) {
        return { completed: false, result: SUPERSEDED };
      }
      if (!transportSessions.trackReader(token, reader)) {
        return { completed: false, result: SUPERSEDED };
      }
      if (!(await bootstrap.sendClientHello(writer, token))) {
        return { completed: false, result: SUPERSEDED };
      }
      const bootstrapResult = await runBootstrapReader({
        reader,
        decode: decodeLengthDelimitedEnvelope,
        isActive: () => transportSessions.isActive(token),
        onMessage: (decoded) => bootstrap.handleBootstrapMessage(decoded, token),
        requestLease: leaseProbe,
        sendLeaseRequest: () => {
          bootstrap.announceProfileActivation(writer);
          return bootstrap.sendLeaseRequest(writer, token);
        },
      });
      if (!transportSessions.isActive(token)) return { completed: false, result: SUPERSEDED };
      if (!bootstrapResult.completed) {
        throw new Error(
          bootstrapResult.welcomed
            ? "bootstrap stream closed after welcome, before it completed"
            : "bootstrap stream closed before ServerWelcome",
        );
      }
      readout.beginTelemetrySession();
      state.connected = true;
      reconnect.notifyBootstrapComplete();
      state.sessionWriter = writer;
      bootstrap.maybeAnnounceProfileActivation();
      control.runSessionStreamReader(reader, token).catch((error) => {
        transportSessions.runIfActive(token, () => log(`session stream reader stopped: ${error}`));
      });
      readout.acceptIncomingUniStreams(transport, token).catch((error) => {
        transportSessions.runIfActive(token, () => log(`incoming uni-stream accept loop error: ${error}`));
      });
      readout.readTelemetryDatagrams(transport, token).catch((error) => {
        transportSessions.runIfActive(token, () => log(`telemetry reader stopped: ${error}`));
      });
      return {
        completed: true,
        leaseGranted: control.authorityFor(motionScope).granted,
        transport,
        token,
        result: { ok: true },
      };
    } catch (error) {
      if (!transportSessions.isActive(token)) return { completed: false, result: SUPERSEDED };
      state.connected = false;
      state.transport = null;
      readout.retireSessionPresentation("failed");
      log(`connect failed: ${error}`);
      transportSessions.close(token);
      const phase = error && error.bootstrapRejected ? "rejected" : "transport";
      return { completed: false, result: { ok: false, failure: { phase } } };
    }
  }

  function handleTransportClosed(token, error) {
    if (!transportSessions.isActive(token)) return;
    state.connected = false;
    state.transport = null;
    state.sessionWriter = null;
    state.resumePendingToken = null;
    state.resumeGimbalLease = false;
    els.resumeBtn.hidden = true;
    releaseTracker.abandon();
    readout.retireSessionPresentation(error === null ? "disconnected" : "failed");
    log(error === null ? "WebTransport session closed" : `WebTransport session errored: ${error}`);
    transportSessions.retire(token);
    void refreshSessionConfig();
    reconnect.notifyDropped({ phase: "transport" });
  }

  const reconnect = createReconnectController({
    connect,
    schedule: (delayMs, callback) => setTimeout(callback, delayMs),
    cancel: (handle) => clearTimeout(handle),
    isVisible: () => document.visibilityState === "visible",
    isActive: () => transportSessions.active !== null,
    random: () => Math.random(),
    log,
  });

  return {
    applyUrlParams,
    connect,
    handleTransportClosed,
    reconnect,
    refreshSessionConfig,
  };
}
