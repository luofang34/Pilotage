import { runBootstrapReader } from "./bootstrap.js";
import { negotiateSessionAuthority } from "./connect-authority.js";
import { createReconnectController } from "./reconnect.js";
import {
  applySessionTargetParams,
  createJoinCredentials,
  failureAfterClose,
  sessionConnectionParameters,
  transportCloseFailure,
} from "./session-admission.js";
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
  const credentials = createJoinCredentials();
  let latestToken = null;
  const discovery = {
    launcherPinned: new URLSearchParams(window.location.search).get("autoconnect") === "1",
    manifestSeen: false,
    endReported: false,
  };

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

  async function connect({ manual } = { manual: true }) {
    const parameters = sessionConnectionParameters(els, credentials, manual, log);
    if (!parameters) return { ok: false, failure: { phase: "construct" } };
    const session = await negotiateSessionAuthority({
      manual,
      gate: controlGate,
      releases: releaseTracker,
      openAndBootstrap: (leaseProbe) =>
        openTransportSession({ ...parameters, manual, leaseProbe }),
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

  async function openTransportSession({ url, certHash, certHashHex, joinToken, manual, leaseProbe }) {
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
    latestToken = token;
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

    const closed = transport.closed.then(
      (info) => {
        handleTransportClosed(token, null, info);
        return transportCloseFailure(info);
      },
      (error) => {
        handleTransportClosed(token, error);
        return transportCloseFailure(error);
      },
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
      if (!(await bootstrap.sendClientHello(writer, token, joinToken))) {
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
      if (latestToken !== token) return { completed: false, result: SUPERSEDED };
      const failure = await failureAfterClose(closed, transportCloseFailure(error));
      if (latestToken !== token) return { completed: false, result: SUPERSEDED };
      state.connected = false;
      state.transport = null;
      readout.retireSessionPresentation("failed");
      log(failure.kind === "admission"
        ? "Join code was rejected. Check the code and press Connect."
        : `connect failed: ${error}`);
      transportSessions.close(token);
      return { completed: false, result: { ok: false, failure } };
    }
  }

  function handleTransportClosed(token, error, info = null) {
    if (latestToken !== token) return;
    const failure = transportCloseFailure(info ?? error);
    if (!transportSessions.isActive(token)) {
      if (failure.phase === "rejected") reconnect.notifyDropped(failure);
      return;
    }
    state.connected = false;
    state.transport = null;
    state.sessionWriter = null;
    state.resumePendingToken = null;
    state.resumeGimbalLease = false;
    els.resumeBtn.hidden = true;
    releaseTracker.abandon();
    readout.retireSessionPresentation(error === null ? "disconnected" : "failed");
    log(failure.phase === "rejected"
      ? "Join code was rejected. Check the code and press Connect."
      : error === null ? "WebTransport session closed" : `WebTransport session errored: ${error}`);
    transportSessions.retire(token);
    void refreshSessionConfig();
    reconnect.notifyDropped(failure);
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
    applyUrlParams: () => applySessionTargetParams(els, window.location.search),
    connect,
    handleTransportClosed,
    reconnect,
    refreshSessionConfig,
  };
}
