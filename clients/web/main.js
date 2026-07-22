// Pilotage demo browser viewer (ADR-0004 local-demo shortcut; ADR-0005
// WebTransport as the primary real-time transport). Not the v1 client — a
// minimal, self-contained page proving command uplink, telemetry downlink,
// and MJPEG video downlink work end to end against a real session host.
//
// Serve statically, no build step:
//   python3 -m http.server 8000
// then open http://localhost:8000/clients/web/index.html
//
// Bootstrap sequence (ADR-0005): open one WebTransport session pinned to the
// host's dev cert hash, open a bidi stream, send ClientHello, read
// ServerWelcome, send a LeaseRequest for vehicle.motion, read LeaseResponse.
// Then: accept host-initiated uni streams and dispatch on their leading
// kind-tag byte (0x01 authority-events, 0x03 one video frame with a capture
// identity header, 0x02 the legacy video frame); read telemetry-fast datagrams
// for live pose; send control-fast datagrams (bare Envelope, ControlFrame arm)
// from arrow/WASD key state.

import {
  encodeClientHelloEnvelope,
  encodeLeaseRequestEnvelope,
  encodeLeaseReleaseEnvelope,
  encodeControlFrameEnvelope,
  encodeControlActionCommandEnvelope,
  encodeProfileActivationEnvelope,
  decodeLengthDelimitedEnvelope,
  STREAM_KIND_AUTHORITY,
  STREAM_KIND_VIDEO,
  STREAM_KIND_VIDEO_V2,
  CONTROL_ACTION,
  MODE_TARGET,
} from "./wire.js";
// Real-time wire decode compiles from the host's own Rust definitions
// (ADR-0014, ADR-0020): the v2 video body (parsed and validated against the
// capture-identity contract) and the telemetry datagram both decode through
// wasm, so their byte/field layouts can never drift from the producer. JS keeps
// only transport plumbing and canvas paint. The one-time bootstrap handshake
// stays on the JS length-delimited reader.
import { decodeVideoFrameV2, decodeDatagramEnvelope } from "./instrument-runtime.js";
import { VideoIdentityTracker } from "./video-identity.js";
import { H264CanvasDecoder, H264DecoderRegistry, FOURCC_H264 } from "./video-h264.js";
import { loadInstruments, PANEL } from "./instruments.js";
import {
  coverInstrumentFailures,
  createDomFaultPresenter,
  failInstrumentSet,
  PanelHealth,
  REASON,
  renderInstrumentSet,
  startDisplayLoop,
  tickInstrumentSet,
} from "./instrument-health.js";
import { TurnDerivation } from "./turn-derivation.js";
import { formatTelemetrySummary, setTelemetrySessionState } from "./telemetry-display.js";
import { AvionicsIngress, FcStateTracker, INCARNATION_POLICY } from "./telemetry-ingress.js";
import { TransportSessionLifecycle } from "./transport-session.js";
import { runBootstrapReader } from "./bootstrap.js";
import { createControlGate } from "./control-gate.js";
import { createReleaseTracker } from "./lease-release.js";
import {
  ACTION_TIMEOUT_MS,
  createActionTracker,
  enqueueAction,
  expirePending,
  pendingModeTarget,
  resolveAction,
} from "./action-tracker.js";
import { negotiateSessionAuthority } from "./connect-authority.js";
import { SnapshotAssociator, associateIfAccepted } from "./snapshot-association.js";
import { CalibrationRegistry, loadCalibrationRegistry } from "./calibration.js";
import { readinessTransition, shouldLogReadFailure } from "./video-diagnostics.js";
import { bindVideoTargets, resolveVideoTarget } from "./video-routing.js";
import { runIncomingStreamAcceptLoop } from "./uni-stream-accept.js";
import { createReconnectController } from "./reconnect.js";
import {
  applySessionConfig,
  launcherSessionOver,
  validSessionConfig,
  whenVisible,
} from "./session-discovery.js";
import { startDatagramControl } from "./datagram-control.js";
import { loadControlShell } from "./control-shell.js";
import { advanceMotionLease, applyMotionRecovery } from "./motion-lease.js";
import {
  INTENT_FAMILY_VELOCITY,
  INTENT_FAMILY_ATTITUDE_THRUST,
  INTENT_FAMILY_GIMBAL_RATE,
  intentCapabilityFor as capabilityFor,
  actionAdvertised,
  buildVelocityIntent,
  buildAttitudeThrustIntent,
  buildGimbalRateIntent,
  integrateHeading,
} from "./typed-command.js";
import { readUniStream } from "./uni-stream.js";

const VEHICLE_ID = 1n; // demo fixture: the single Gazebo vehicle this host serves.
const INSTRUMENT_SOURCE_ID = 1n; // explicit simulator adapter source; never first-packet selection.
// Aviate publishes attitude at 50 Hz and kinematics at 30 Hz. This simulator
// profile admits several stream periods plus transport jitter; an aircraft
// profile must derive its own limit from the intended function. The same
// budget bounds status-to-numeric authorization pairing in the ingress.
const SIM_COHERENCE_LIMIT_NS = 300_000_000n;
const MOTION_SCOPE = "vehicle.motion";
// The direct-flight scope (CTRL-01): attitude + collective under its OWN
// lease and generation. The FPV toggle is a SCOPE HANDOVER when a vehicle
// advertises it — never a mode flip reinterpreting velocity numbers.
const DIRECT_SCOPE = "vehicle.motion.direct";
const CONTROL_HZ = 30; // continuous control send rate; superseded samples are droppable (ADR-0011).
const GIMBAL_SCOPE = "vehicle.gimbal"; // gimbal pointing scope (GIM-01): pitch/yaw LOS rate demands, leased separately.
// The simulator lifecycle scope (SIM-01): SimReset lives here — and only
// here — under its own on-demand lease. Flight authority neither grants
// nor implies it, and a live/RF host never advertises it.
const SIM_LIFECYCLE_SCOPE = "sim.lifecycle";

/** The exclusive-authority group a motion-family scope belongs to: both
 *  flight scopes drive one FC and are one authority on the host, whose
 *  link-loss recovery ack names the GROUP key. */
function motionGroup(scope) {
  return scope === DIRECT_SCOPE ? MOTION_SCOPE : scope;
}

const els = {
  host: document.getElementById("host"),
  port: document.getElementById("port"),
  certHash: document.getElementById("certHash"),
  connectBtn: document.getElementById("connectBtn"),
  status: document.getElementById("status"),
  overlay: document.getElementById("overlay"),
  telemetry: document.getElementById("telemetry"),
  gamepad: document.getElementById("gamepad"),
  pfd: document.getElementById("pfd"),
  hsi: document.getElementById("hsi"),
  flightMode: document.getElementById("flightMode"),
};
const pfdCtx = els.pfd.getContext("2d");
const hsiCtx = els.hsi.getContext("2d");
const pfdFaultPresenter = createDomFaultPresenter(els.pfd);
const hsiFaultPresenter = createDomFaultPresenter(els.hsi);

/** Session-scoped mutable state the connect flow and background loops share. */
const state = {
  transport: null,
  sessionId: 0,
  generation: 0n,
  sequence: 0,
  startNanos: BigInt(Date.now()) * 1_000_000n, // arbitrary local monotonic-ish origin for sampled_at (ADR-0009: endpoint-local, never compared raw across endpoints).
  // The runtime owns key bindings AND the held-key state; the shell only
  // forwards transitions. This records which Gamepad.id the runtime last
  // resolved, so selection re-runs only when the device actually changes.
  selectedPadId: null,
  pendingReset: false,
  pendingFpvToggle: false,
  // The FC arm/disarm verdict (kind:result) last logged, so each verdict
  // is announced exactly once however many samples repeat it.
  lastFcVerdictLogged: null,
  // Camera-velocity vs FPV/direct flight, tracked here so the DOM toggle
  // sends an explicit typed ModeRequest TARGET, never a stateless flip.
  // Flips ONLY on the host's accepted mode ack (reliable delivery), never
  // optimistically on send.
  fpvActive: false,
  // Pending discrete presses awaiting their ControlActionResult; each rides
  // every outgoing frame for its scope until acked or timed out.
  actionTracker: createActionTracker(),
  // The activation revision last announced on THIS session's reliable
  // stream, so a device swap (which advances the revision) re-announces
  // before frames carry the new value. Null until the first announcement.
  announcedActivationRevision: null,
  // Which scope currently carries flight: vehicle.motion (velocity) or
  // vehicle.motion.direct (attitude + collective). Swapped only at the
  // release boundary of a scope handover.
  motionScope: MOTION_SCOPE,
  // The scope the in-flight handover switches to when the release acks.
  pendingMotionScope: null,

  // The client-integrated direct-flight heading setpoint (rad, local NED),
  // seeded from telemetry at scope entry; the client OWNS the heading on
  // the direct scope.
  fpvHeading: 0,
  lastDirectFrameMs: 0,
  // The sim-lifecycle scope's on-demand authority: leased just long enough
  // to command a reset, released when the result lands (a held-but-silent
  // scope would only feed the host's silence watchdog).
  lifecycle: { granted: false, generation: 0n, pendingPress: false },
  // The typed capability negotiation from ServerWelcome: every advertised
  // scope with its intent families, limits, and actions. Control fails
  // closed (no motion frames) until the motion scope advertises velocity.
  advertisedScopes: [],
  connected: false,
  leaseGranted: false,
  // The generation the motion lease held when last released: a post-handover
  // regrant must be strictly newer than this fence to be accepted.
  motionFence: 0n,
  // A denied motion reacquire is terminal — the runtime stops re-requesting.
  motionDenied: false,
  // Whether the host has confirmed (via LinkLossCleared) it cleared the
  // vehicle's link-loss latch on the current generation. The runtime keeps
  // neutralizing until this is true, so recovery never rests on a best-effort
  // datagram. Irrelevant in steady flight; reset false when a release starts a
  // new recovery cycle.
  motionRecovered: true,
  // The gimbal scope's own lease/fencing state: independent generation and
  // sequence, granted (or not) without affecting flight control.
  gimbalGeneration: 0n,
  gimbalSequence: 0,
  gimbalLeaseGranted: false,
  gimbalLeaseDenied: false, // a denied scope is not re-requested this session
  // The control runtime owns the request debounce, the R3 baseline, and the
  // gimbal stream latch; the shell holds only the loaded runtime.
  controlShell: null,
  // Advances on each fresh connect so the runtime seeds its edge baselines and
  // a control held across a reconnect fires no edge.
  controlGeneration: 0,
  skippedVideoFrames: 0,
  supersededVideoFrames: 0,
  // Page-lifetime latch: wasm absence never heals without a reload (the
  // instrument module loads once at boot), so the H.264-unavailable notice
  // is worth one line, not one line per frame at stream rate.
  h264UnavailableLogged: false,
  droppedIdentityFrames: 0,
  // Latest capture-to-snapshot association verdict for an accepted frame
  // (ADR-0020). This diagnostic does not authorize a conformal overlay.
  lastAssociation: null,
};
const transportSessions = new TransportSessionLifecycle();
// Capture-identity guard for the video downlink: drops duplicate/reordered/
// stale frames so a replayed frame never displaces a newer one or refreshes
// its age (ADR-0020).
const videoIdentity = new VideoIdentityTracker();
// Bounded history of accepted aircraft snapshots, fed from the telemetry path,
// against which an accepted video frame's capture time is associated (ADR-0020).
const snapshotHistory = new SnapshotAssociator();

// The published, hash-verified camera calibrations that feed the conformal
// gate's recognized set (ADR-0021). Starts empty and fail-closed; the async
// load replaces it once the artifact is fetched and verified, and a fetch or
// verification failure simply leaves it empty (no recognized calibration).
const CALIBRATION_ARTIFACT_URL = "./sim-fpv-calibration.json";
let calibrationRegistry = new CalibrationRegistry();
loadCalibrationRegistry(CALIBRATION_ARTIFACT_URL)
  .then((registry) => {
    calibrationRegistry = registry;
  })
  .catch(() => {
    // Fail closed: keep the empty registry, conformal output stays off.
  });

// Ingestion accepts only source advancement for the selected vehicle. Drawing
// remains on requestAnimationFrame, decoupled from telemetry publication.
// Each panel latches display failures independently; a fault in the shared
// wasm backend (load/ABI/init) fails both.
function newSimulatorAvionicsIngress() {
  return new AvionicsIngress({
    vehicleId: VEHICLE_ID,
    sourceId: INSTRUMENT_SOURCE_ID,
    // SIM-only policy permits a bounded number of unseen attachment tokens.
    // Aircraft profiles pin a source-issued incarnation at authenticated
    // bootstrap and do not infer transitions from telemetry.
    incarnationPolicy: INCARNATION_POLICY.SIM_ACCEPT_UNSEEN,
    maximumSkewNanos: SIM_COHERENCE_LIMIT_NS,
  });
}

function retireSessionPresentation(phase) {
  instruments.ingress = newSimulatorAvionicsIngress();
  // FC-state freshness is session-scoped: a new session must not
  // inherit the old session's arm report or its age.
  instruments.fcState = new FcStateTracker();
  // Drop the previous session's snapshot history so a new session can never
  // associate a video frame against a stale snapshot from the old one.
  snapshotHistory.reset();
  turnDerivation.reset();
  // Close every H.264 decoder so none outlives the session that owned it; a
  // reconnect rebuilds them bound to the new session token.
  h264Registry?.closeAll();
  setTelemetrySessionState(els, phase);
}

// Browser watchdog cadence (simulator-only): a scheduling domain separate
// from requestAnimationFrame, so a stalled render loop still trips the
// liveness deadline and covers the stale frame. Passed to each panel's
// health so a tick arriving late against this cadence is recognized as
// page-wide scheduling starvation rather than a render-pipeline fault.
const WATCHDOG_INTERVAL_MS = 250;

const instruments = {
  mod: null,
  moduleFault: null,
  ingress: newSimulatorAvionicsIngress(),
  fcState: new FcStateTracker(),
  health: {
    [PANEL.PFD]: new PanelHealth({ tickIntervalMs: WATCHDOG_INTERVAL_MS }),
    [PANEL.HSI]: new PanelHealth({ tickIntervalMs: WATCHDOG_INTERVAL_MS }),
  },
};

/** The two instrument paint targets and their independent fault surfaces. */
function instrumentTargets() {
  return [
    [PANEL.PFD, pfdCtx, els.pfd, pfdFaultPresenter],
    [PANEL.HSI, hsiCtx, els.hsi, hsiFaultPresenter],
  ];
}

function log(line) {
  const time = new Date().toISOString().split("T")[1].replace("Z", "");
  els.status.textContent = `[${time}] ${line}\n${els.status.textContent}`.slice(0, 8000);
}

function nowNanos() {
  return state.startNanos + BigInt(Math.round(performance.now() * 1_000_000));
}

/** Parses the URL's `?host=&port=&cert=` params into the input boxes, if present. */
function applyUrlParams() {
  const params = new URLSearchParams(window.location.search);
  if (params.has("host")) els.host.value = params.get("host");
  if (params.has("port")) els.port.value = params.get("port");
  if (params.has("cert")) els.certHash.value = params.get("cert");
}

// The manifest-discovery context: whether this page came from the launcher
// (a launcher-pinned URL, or a manifest served earlier), and whether the
// session's end has already been reported (re-armed by the next manifest,
// so a NEW session's later teardown reports again).
const discovery = {
  launcherPinned: new URLSearchParams(window.location.search).get("autoconnect") === "1",
  manifestSeen: false,
  endReported: false,
};

/** Re-reads the launcher-served session manifest and updates the connect
 *  inputs, so a stale tab (its URL pins an older session's certificate)
 *  converges on the CURRENT session at its next attempt instead of
 *  retrying a dead hash forever. A missing manifest on a page served
 *  without the launcher is silent — there is nothing to converge to. In a
 *  launcher context, a manifest that is now GONE means the launcher
 *  session ended (it deletes `session.json` at teardown): that is said
 *  once, so the retry loop — kept running, because it converges onto the
 *  next session's manifest by itself — is never mistaken for a live host. */
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
    // A served-but-malformed manifest is an invalid config, not a dead
    // launcher: stay silent like any other unusable manifest.
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

/** Decodes a lowercase-hex cert hash string into a Uint8Array digest. */
function hexToBytes(hex) {
  const clean = hex.trim().toLowerCase();
  const out = new Uint8Array(clean.length / 2);
  for (let i = 0; i < out.length; i += 1) {
    out[i] = Number.parseInt(clean.substr(i * 2, 2), 16);
  }
  return out;
}

// A superseded attempt (a newer session token began) returns this so the
// reconnect controller does not treat it as a failure and reschedule — the
// newer flow owns recovery.
const SUPERSEDED = { ok: true };

// Opening one session. `manual` is true only for a user-initiated Connect:
// only then is the motion lease requested and control resumed. An automatic
// reconnect (`manual: false`) restores transport, telemetry, and video but
// NEVER re-requests motion authority — control stays suspended until the user
// explicitly reconnects. Returns `{ ok }` / `{ ok: false, failure }` for the
// reconnect controller.
async function connect({ manual } = { manual: true }) {
  const host = els.host.value.trim();
  const port = els.port.value.trim();
  const certHashHex = els.certHash.value.trim();
  if (!host || !port || !certHashHex) {
    log("host, port, and cert hash are all required");
    return { ok: false, failure: { phase: "construct" } };
  }
  // A SHA-256 hash is exactly 32 bytes; anything else is a configuration
  // error (non-retryable), not something WebTransport should be handed.
  if (!/^[0-9a-fA-F]{64}$/.test(certHashHex)) {
    log("certificate hash must be exactly 64 hexadecimal characters (SHA-256)");
    return { ok: false, failure: { phase: "construct" } };
  }
  const url = `https://${host}:${port}/pilotage`;
  const certHash = hexToBytes(certHashHex);

  // The gate re-arm, release-settlement wait, live lease probe, and
  // post-grant decision run inside the production orchestration module,
  // which the lifecycle tests execute directly.
  const session = await negotiateSessionAuthority({
    manual,
    gate: controlGate,
    releases: releaseTracker,
    openAndBootstrap: (leaseProbe) =>
      openTransportSession({ url, certHash, certHashHex, manual, leaseProbe }),
    startControl: ({ transport, token }) => startControlLoop(transport, token),
    controlUnavailable: ({ token }) => {
      transportSessions.runIfActive(token, () => {
        state.leaseGranted = false;
        sendLeaseRelease(token);
      });
    },
    releaseLease: ({ token }) => {
      transportSessions.runIfActive(token, () => {
        state.leaseGranted = false;
        sendLeaseRelease(token);
        els.gamepad.textContent =
          "control released — input lost during connect; press Connect to resume";
        log("input lost during connect — lease released immediately");
      });
    },
    telemetryOnly: (_session, wasManual) => {
      // The overlay carries this state too: buried in the collapsed log,
      // a lease-less session is indistinguishable from a healthy wait for
      // telemetry, and every arm press would then vanish behind it (#180).
      if (wasManual) {
        // A telemetry-only vehicle (e.g. the Aviate adapter, ADR-0018)
        // advertises no controllable scopes; sending control frames
        // anyway would only generate a 30 Hz stream of rejections.
        els.overlay.textContent = "no control lease — telemetry/video only; press Connect to take control";
        log("no control lease granted; viewer is telemetry/video only");
      } else {
        els.overlay.textContent = "reconnected without control; press Connect to resume";
        log("reconnected (telemetry/video); press Connect to resume control");
      }
    },
  });
  // A failed attempt may mean this tab belongs to an older session (its
  // pinned certificate no longer matches the host on this port): re-read
  // the launcher manifest so the NEXT attempt targets the live session.
  // Config errors are excluded — they need the user, not a retry target.
  if (!session.result.ok && session.result.failure?.phase !== "construct") {
    void refreshSessionConfig();
  }
  return session.result;
}

/** Transport construction, handshake, and bootstrap for one session
 *  attempt. Resolves to `{ completed, leaseGranted, transport, token,
 *  result }`; `result` is connect()'s return contract for the reconnect
 *  controller (SUPERSEDED, ok, or a classified failure). */
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
    state.generation = 0n;
    state.sequence = 0;
    state.motionFence = 0n;
    state.motionDenied = false;
    state.motionRecovered = true;
    state.gimbalGeneration = 0n;
    state.gimbalSequence = 0;
    state.gimbalLeaseGranted = false;
    state.gimbalLeaseDenied = false;
    // A fresh session generation: the control runtime seeds its edge baselines
    // on the first tick, so a control held across the reconnect fires no edge.
    state.controlGeneration = (state.controlGeneration + 1) >>> 0;
    state.pendingReset = false;
    state.pendingFpvToggle = false;
    state.lastFcVerdictLogged = null;
    state.connected = false;
    state.leaseGranted = false;
    state.skippedVideoFrames = 0;
    resetVideoDiagnostics();
    retireSessionPresentation("connecting");
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
    if (!transportSessions.trackWriter(token, writer)) return { completed: false, result: SUPERSEDED };
    if (!transportSessions.trackReader(token, reader)) return { completed: false, result: SUPERSEDED };

    if (!(await sendClientHello(writer, token))) return { completed: false, result: SUPERSEDED };
    // Only a manual connect requests the motion lease; an auto-reconnect
    // completes bootstrap at ServerWelcome, leaving control suspended.
    const bootstrap = await runBootstrapReader({
      reader,
      decode: decodeLengthDelimitedEnvelope,
      isActive: () => transportSessions.isActive(token),
      onMessage: (decoded) => handleBootstrapMessage(decoded, token),
      // The orchestration module's live probe, evaluated at the
      // ServerWelcome moment immediately before the one LeaseRequest
      // emission: a blur latched during ANY await of this connect
      // suppresses the request entirely.
      requestLease: leaseProbe,
      sendLeaseRequest: () => {
        // The activation announcement precedes the LeaseRequest on the SAME
        // ordered stream: the host records the profile before it can grant
        // the lease, so no accepted frame ever precedes its traceability
        // record (INPUT-01).
        announceProfileActivation(writer);
        return sendLeaseRequest(writer, token);
      },
    });
    if (!transportSessions.isActive(token)) return { completed: false, result: SUPERSEDED };
    if (!bootstrap.completed) {
      // An untyped stream end is RETRYABLE wherever it lands in the
      // handshake: the host's Close carries no wire payload today, so a
      // pre-welcome EOF is indistinguishable from an ordinary early
      // transport drop, and classifying it as a rejection would
      // permanently stop recovery after a network blip. Only a TYPED
      // protocol rejection may set `bootstrapRejected` (none exists on
      // the wire yet — tracked in the reconnection issue).
      throw new Error(
        bootstrap.welcomed
          ? "bootstrap stream closed after welcome, before it completed"
          : "bootstrap stream closed before ServerWelcome",
      );
    }

    // Negotiation is the lifecycle boundary for measurement ordering. The
    // token prevents readers from the replaced transport from reaching this
    // newly empty ingress even if their promises settle later.
    instruments.ingress = newSimulatorAvionicsIngress();
    instruments.fcState = new FcStateTracker();
    setTelemetrySessionState(els, "awaiting");
    state.connected = true;
    // Bootstrap is proven good only here — reset the reconnect backoff now, not
    // at transport.ready (which precedes bootstrap).
    reconnect.notifyBootstrapComplete();
    state.sessionWriter = writer;
    // An auto-reconnect skips the lease path (and its announcement); the
    // session still needs the traceability record before any later manual
    // lease request.
    maybeAnnounceProfileActivation();
    runSessionStreamReader(reader, token).catch((error) => {
      transportSessions.runIfActive(token, () => log(`session stream reader stopped: ${error}`));
    });
    acceptIncomingUniStreams(transport, token).catch((error) => {
      transportSessions.runIfActive(token, () => log(`incoming uni-stream accept loop error: ${error}`));
    });
    readTelemetryDatagrams(transport, token).catch((error) => {
      transportSessions.runIfActive(token, () => log(`telemetry reader stopped: ${error}`));
    });
    return {
      completed: true,
      leaseGranted: state.leaseGranted,
      transport,
      token,
      result: { ok: true },
    };
  } catch (error) {
    if (!transportSessions.isActive(token)) return { completed: false, result: SUPERSEDED };
    state.connected = false;
    state.transport = null;
    retireSessionPresentation("failed");
    log(`connect failed: ${error}`);
    transportSessions.close(token);
    // "rejected" (non-retryable) is reserved for a TYPED protocol
    // rejection or authenticated close code; an untyped failure — EOF,
    // reset, timeout — is always a retryable transport drop.
    const phase = error && error.bootstrapRejected ? "rejected" : "transport";
    return { completed: false, result: { ok: false, failure: { phase } } };
  }
}

function handleTransportClosed(token, error) {
  if (!transportSessions.isActive(token)) return;
  state.connected = false;
  state.transport = null;
  state.sessionWriter = null;
  // An unacknowledged release rides down with the transport; the host
  // watchdog covers it from here.
  releaseTracker.abandon();
  retireSessionPresentation(error === null ? "disconnected" : "failed");
  log(error === null ? "WebTransport session closed" : `WebTransport session errored: ${error}`);
  transportSessions.retire(token);
  // Every transport death funnels through here — including a handshake
  // failure, whose `closed` rejection retires the token BEFORE connect()'s
  // own catch can see a live session. Re-read the launcher manifest so the
  // scheduled retry targets the CURRENT session, not a dead certificate.
  void refreshSessionConfig();
  // The drop was not user-initiated (there is no disconnect control); recover
  // the transport if the user still wants a session. A clean or errored close
  // is treated as a transient transport drop — retried with capped, jittered
  // backoff. Control is NOT resumed by the reconnect (see connect: manual).
  reconnect.notifyDropped({ phase: "transport" });
}

// Input-loss gate (CTRL-04): blur LATCHES loss synchronously so a
// blur/refocus between control-loop ticks can never be missed; only an
// explicit new connect re-arms it.
const controlGate = createControlGate({ isFocused: () => document.hasFocus() });

// One in-flight explicit lease release and its acknowledgement; an
// immediate reconnect waits (bounded) for settlement so it cannot race
// into an AlreadyHeld denial. The host watchdog stays the backup.
const releaseTracker = createReleaseTracker();

/** Auto-recovery for an unexpectedly dropped session: reconnects the transport
 *  (telemetry/video) with capped, jittered backoff, and never re-requests
 *  motion authority. Uses real timers, page visibility, and session state. */
const reconnect = createReconnectController({
  connect,
  schedule: (delayMs, cb) => setTimeout(cb, delayMs),
  cancel: (handle) => clearTimeout(handle),
  isVisible: () => document.visibilityState === "visible",
  isActive: () => transportSessions.active !== null,
  random: () => Math.random(),
  log,
});

/** Writes a length-delimited `ClientHello` envelope onto the bootstrap bidi stream. */
async function sendClientHello(writer, token) {
  if (!transportSessions.isActive(token)) return false;
  const hello = encodeClientHelloEnvelope({
    protocolVersion: 1,
    clientName: "pilotage-web-viewer",
    joinToken: new Uint8Array(0),
  });
  await writer.write(lengthDelimit(hello));
  if (!transportSessions.isActive(token)) return false;
  log("sent ClientHello");
  return true;
}

/** Writes a length-delimited `LeaseRequest` for the motion scope. */
async function sendLeaseRequest(writer, token) {
  if (!transportSessions.isActive(token)) return false;
  const request = encodeLeaseRequestEnvelope({ vehicleId: VEHICLE_ID, scope: MOTION_SCOPE });
  await writer.write(lengthDelimit(request));
  if (!transportSessions.isActive(token)) return false;
  log(`sent LeaseRequest for ${MOTION_SCOPE}`);
  return true;
}

/** Sends a `LeaseRelease` for the motion scope on the reliable session
 *  stream and starts the bounded wait for the host's acknowledgement. */
function sendLeaseRelease(token) {
  const writer = state.sessionWriter;
  if (!writer || !transportSessions.isActive(token)) return;
  if (state.gimbalLeaseGranted) {
    // Fire-and-forget: the gimbal scope has no motion authority, so its
    // release rides best-effort alongside the tracked motion release
    // (the host watchdog covers a lost one).
    state.gimbalLeaseGranted = false;
    const gimbalRelease = encodeLeaseReleaseEnvelope({
      vehicleId: VEHICLE_ID,
      scope: GIMBAL_SCOPE,
    });
    writer.write(lengthDelimit(gimbalRelease)).catch(() => {});
    log("sent LeaseRelease for " + GIMBAL_SCOPE);
  }
  if (releaseTracker.isPending()) return; // one release per loss; the ack settles it
  // Input loss releases whichever scope currently carries flight.
  const scope = state.motionScope;
  const release = encodeLeaseReleaseEnvelope({ vehicleId: VEHICLE_ID, scope });
  const settled = releaseTracker.begin();
  settled.then((outcome) => {
    transportSessions.runIfActive(token, () => log(`lease release ${outcome}`));
  });
  writer.write(lengthDelimit(release)).catch(() => {
    // The reliable stream refused the write: the transport is dying, and
    // the host watchdog will do the release.
    releaseTracker.abandon();
  });
  log("sent LeaseRelease for " + scope);
}

/** Applies a motion-scope reliable-stream message to the flat authority state
 *  through the pure {@link advanceMotionLease} transition: it enforces the
 *  generation fence and terminal denial, and installs a fresh generation
 *  (restarting the sequence before any frame rides it) only when the grant
 *  clears the fence. Returns the resulting authority for the caller to log. */
function applyMotionLease(decoded) {
  const before = {
    granted: state.leaseGranted,
    generation: state.generation,
    fence: state.motionFence,
    denied: state.motionDenied,
  };
  const after = advanceMotionLease(before, decoded);
  state.leaseGranted = after.granted;
  state.motionFence = after.fence;
  state.motionDenied = after.denied;
  if (after.generation !== before.generation) {
    state.generation = after.generation;
    state.sequence = 0;
  }
  return after;
}

/** Reads the session (bootstrap) stream after the handshake completes:
 *  the host's `LeaseReleased` acknowledgement arrives here. */
async function runSessionStreamReader(reader, token) {
  let pending = new Uint8Array(0);
  for (;;) {
    const { value, done } = await reader.read();
    if (!transportSessions.isActive(token) || done) return;
    pending = appendBytes(pending, value);
    for (;;) {
      const decoded = decodeLengthDelimitedEnvelope(pending);
      if (!decoded) break;
      pending = pending.subarray(decoded.consumed);
      if (decoded.kind === "ControlActionResult") {
        handleActionResult(decoded.message);
      }
      if (decoded.kind === "LeaseReleased") {
        // Validate the acknowledgement before it settles anything: an
        // ack for another vehicle or scope proves nothing about ours.
        // Wire vehicle ids decode as BigInt; comparing through Number
        // against the BigInt constant is always false.
        const m = decoded.message;
        if (m.vehicleId === VEHICLE_ID && m.scope === state.motionScope) {
          releaseTracker.acknowledge();
          // The motion lease is released (input-loss OR a profile handover):
          // drop local authority and fence the released generation so the
          // runtime gates motion output and only re-requests once reflected. A
          // release opens a new recovery cycle, so the host's prior
          // confirmation no longer applies until it clears the fresh generation.
          applyMotionLease(decoded);
          state.motionRecovered = false;
          log(`LeaseReleased: released=${m.released} generation=${m.generation}`);
          if (state.pendingMotionScope) {
            // The scope handover's release boundary: flight moves to the
            // sibling scope. Both siblings share ONE authority group on the
            // host — one generation domain (the fence carries over) and one
            // link-loss latch, so recovery is ALWAYS required and its ack
            // names the group.
            state.motionScope = state.pendingMotionScope;
            state.pendingMotionScope = null;
            if (state.motionScope === DIRECT_SCOPE) {
              const heading = currentTelemetryHeading();
              state.fpvHeading = heading ?? 0;
              state.lastDirectFrameMs = 0;
            }
            log(`motion scope is now ${state.motionScope}`);
          }
        } else if (m.vehicleId === VEHICLE_ID && m.scope === GIMBAL_SCOPE) {
          log(`LeaseReleased[gimbal]: released=${m.released} generation=${m.generation}`);
        } else {
          log(`ignoring LeaseReleased for vehicle=${m.vehicleId} scope=${m.scope}`);
        }
      }
      if (
        decoded.kind === "LeaseResponse" &&
        decoded.message.scope === GIMBAL_SCOPE &&
        decoded.message.vehicleId === VEHICLE_ID
      ) {
        // The gimbal lease is requested after bootstrap, so its response
        // arrives here rather than in the bootstrap reader. Another
        // vehicle's response must never install our generation.
        const m = decoded.message;
        state.gimbalGeneration = BigInt(m.generation || 0);
        state.gimbalLeaseGranted = !!m.granted;
        state.gimbalLeaseDenied = !m.granted;
        log(`LeaseResponse[gimbal]: granted=${m.granted} generation=${m.generation}`);
        if (!m.granted) {
          log(`gimbal lease denied (reason ${m.reason}); flight control is unaffected`);
        }
      }
      if (
        decoded.kind === "LeaseResponse" &&
        decoded.message.scope === SIM_LIFECYCLE_SCOPE &&
        decoded.message.vehicleId === VEHICLE_ID
      ) {
        const m = decoded.message;
        state.lifecycle.granted = !!m.granted;
        state.lifecycle.generation = BigInt(m.generation || 0);
        log(`LeaseResponse[lifecycle]: granted=${m.granted} generation=${m.generation}`);
        if (m.granted && state.lifecycle.pendingPress) {
          state.lifecycle.pendingPress = false;
          if (requestAction(SIM_LIFECYCLE_SCOPE, CONTROL_ACTION.simReset)) {
            log("simulation reset requested");
          }
        } else if (!m.granted) {
          state.lifecycle.pendingPress = false;
          log(`sim reset authorization denied (reason ${m.reason})`);
        }
      }
      if (
        decoded.kind === "LeaseResponse" &&
        decoded.message.scope === state.motionScope &&
        decoded.message.vehicleId === VEHICLE_ID
      ) {
        // A motion lease response after a profile handover. The pure transition
        // enforces the fence (a grant must be strictly newer than the released
        // generation) and the terminal denial; a fresh grant installs the new
        // generation and restarts the sequence BEFORE any frame rides it, so
        // the runtime resumes only on verified authority. Another vehicle's
        // response never installs ours (filtered above).
        const m = decoded.message;
        const after = applyMotionLease(decoded);
        if (after.stale !== undefined) {
          log(`ignoring stale motion grant generation=${m.generation} (fence ${after.fence})`);
        } else if (after.denied) {
          log(`motion lease denied (reason ${m.reason}); control stays suspended`);
        } else {
          // Direct-flight state follows the GRANTED scope, never a local
          // optimistic flip.
          state.fpvActive = state.motionScope === DIRECT_SCOPE;
          els.overlay.textContent = state.fpvActive
            ? "direct (FPV) flight scope granted"
            : "camera flight scope granted";
          log(`LeaseResponse[motion:${state.motionScope}]: granted generation=${m.generation}`);
        }
      }
    }
  }
}

/** Prefixes an already-encoded `Envelope` with a protobuf varint byte-length, matching `encode_length_delimited` on the host. */
function lengthDelimit(envelopeBytes) {
  const prefix = [];
  let v = envelopeBytes.length;
  for (;;) {
    let byte = v & 0x7f;
    v >>>= 7;
    if (v !== 0) {
      prefix.push(byte | 0x80);
    } else {
      prefix.push(byte);
      break;
    }
  }
  const out = new Uint8Array(prefix.length + envelopeBytes.length);
  out.set(prefix, 0);
  out.set(envelopeBytes, prefix.length);
  return out;
}


function handleBootstrapMessage(decoded, token) {
  if (!transportSessions.isActive(token)) return;
  if (decoded.kind === "ServerWelcome") {
    state.sessionId = decoded.message.sessionId;
    state.advertisedScopes = decoded.message.advertisedScopes ?? [];
    // Correlation ids are per-connection; presses from a dead session must
    // not retransmit into a new one, and the new session has announced
    // nothing yet. Flight restarts on the velocity scope.
    state.actionTracker = createActionTracker();
    state.announcedActivationRevision = null;
    state.motionScope = MOTION_SCOPE;
    state.pendingMotionScope = null;
    state.fpvActive = false;
    state.lifecycle = { granted: false, generation: 0n, pendingPress: false };
    log(`ServerWelcome: session=${decoded.message.sessionId} principal=${decoded.message.principalId}`);
    for (const scope of state.advertisedScopes) {
      const families = scope.intents.map((i) => i.family).join(",");
      log(`capability: ${scope.scope} intents=[${families}] actions=${scope.actions.length}`);
    }
    if (!velocityCapabilityFor(MOTION_SCOPE)) {
      log("vehicle advertises no velocity intent for vehicle.motion; motion control disabled");
    }
  } else if (decoded.kind === "ControlActionResult") {
    handleActionResult(decoded.message);
  } else if (decoded.kind === "LeaseResponse") {
    // Bootstrap requests only the motion lease, but route by scope
    // anyway: a response for any other scope must not overwrite the
    // motion generation.
    if (decoded.message.scope && decoded.message.scope !== MOTION_SCOPE) {
      log(`ignoring bootstrap LeaseResponse for scope=${decoded.message.scope}`);
      return;
    }
    state.generation = BigInt(decoded.message.generation || 0);
    state.leaseGranted = !!decoded.message.granted;
    log(`LeaseResponse: granted=${decoded.message.granted} generation=${decoded.message.generation}`);
    if (!decoded.message.granted) {
      els.overlay.textContent = `lease denied (reason ${decoded.message.reason})`;
    }
  }
}

function appendBytes(existing, incoming) {
  const out = new Uint8Array(existing.length + incoming.length);
  out.set(existing, 0);
  out.set(incoming, existing.length);
  return out;
}

/** Coalesces uni-stream read failures into at most one log line per interval
 * (after the first, which logs immediately): a host that resets a stalled
 * frame's stream (ADR video deadline) surfaces a WebTransportError here every
 * frame, which after the first is expected, not per-frame news. */
function noteStreamReadFailure(error) {
  streamReadFailures.count += 1;
  const nowMs = performance.now();
  if (
    !shouldLogReadFailure(
      nowMs,
      streamReadFailures.lastLoggedMs,
      STREAM_READ_FAILURE_LOG_INTERVAL_MS,
    )
  ) {
    return;
  }
  streamReadFailures.lastLoggedMs = nowMs;
  log(`uni stream read failed: ${error} (${streamReadFailures.count} total this session)`);
}

/** Runs the one incoming-uni-stream accept loop for this session. An individual
 * received stream failing loses only that frame and the loop keeps accepting;
 * the collection stream itself closing or erroring is terminal for the whole
 * WebTransport session (a re-acquired reader is handed the same stored error),
 * so it is surfaced once as a session failure and never reacquired. */
async function acceptIncomingUniStreams(transport, token) {
  await runIncomingStreamAcceptLoop(transport.incomingUnidirectionalStreams, {
    isActive: () => transportSessions.isActive(token),
    handleStream: (stream) => readOneUniStream(stream, token),
    onStreamFailure: (error) =>
      transportSessions.runIfActive(token, () => noteStreamReadFailure(error)),
    onCollectionTerminal: (error) =>
      transportSessions.runIfActive(token, () => handleTransportClosed(token, error)),
    trackReader: (reader) => transportSessions.trackReader(token, reader),
    untrackReader: (reader) => transportSessions.untrackReader(token, reader),
  });
}

/** Drains one uni stream to completion via the shared {@link readUniStream}
 *  core (kind tag + live authority dispatch), then renders a video body at
 *  close. */
async function readOneUniStream(stream, token) {
  if (!transportSessions.isActive(token)) return;
  const reader = stream.getReader();
  if (!transportSessions.trackReader(token, reader)) return;
  try {
    const { kind, tail, aborted } = await readUniStream(reader, {
      authorityKind: STREAM_KIND_AUTHORITY,
      decode: decodeLengthDelimitedEnvelope,
      onAuthorityEnvelope: (decoded) => dispatchAuthorityEnvelope(decoded, token),
      shouldContinue: () => transportSessions.isActive(token),
    });
    if (aborted) return;
    // Video streams are per-frame: the whole body is one frame, rendered at
    // close. (Authority already dispatched incrementally inside readUniStream.)
    if (kind === STREAM_KIND_VIDEO_V2) {
      await renderVideoFrameV2(tail, token);
    } else if (kind === STREAM_KIND_VIDEO) {
      await renderVideoFrame(tail, token);
    } else if (kind !== null && kind !== STREAM_KIND_AUTHORITY) {
      log(`unrecognized uni stream kind tag 0x${kind.toString(16)}`);
    }
  } finally {
    transportSessions.untrackReader(token, reader);
  }
}

/** The dedicated authority-events stream is opened once at connection start and may carry several length-delimited envelopes over the stream's lifetime; decode every complete one buffered. */
/** Handles one authority-stream envelope, decoded live as it arrives. */
function dispatchAuthorityEnvelope(decoded, token) {
  if (!transportSessions.isActive(token)) return;
  if (decoded.kind === "AuthorityEvent") {
    els.overlay.textContent = `authority: ${decoded.message.arm}`;
    log(`authority event: ${decoded.message.arm}`);
  } else if (decoded.kind === "LinkLossCleared") {
    // The host confirmed it cleared the vehicle's link-loss latch. Resume live
    // control only when it correlates to OUR pending recovery (vehicle + motion
    // scope + the current fresh generation) — the shared transition the
    // uni-stream test drives.
    if (applyMotionRecovery(decoded, state, VEHICLE_ID, motionGroup(state.motionScope))) {
      log(`LinkLossCleared[motion]: recovery confirmed on generation=${decoded.message.generation}`);
    }
  }
}

// The source_id (0 = onboard FPV, 1 = chase, 2 = gimbal payload) routes a frame
// to its canvas. An unknown source_id is counted and logged, never a hard
// failure, so a host streaming a source this viewer lacks degrades
// gracefully. Codec dispatch is
// separate: "MJPG" paints via createImageBitmap; "H264" routes to WebCodecs.
const FOURCC_MJPEG = "MJPG";
const VIDEO_TARGETS = bindVideoTargets(document);

// Per-source H.264 decoders, each bound to the transport session that built it
// (H264DecoderRegistry owns that lifetime — see video-h264.js). A source that
// only ever carries MJPEG never constructs one. The decoder's output callback
// tests its own session's liveness, so a reconnect's fresh decoder supersedes
// any retired one. The registry's ownership table lives in the instrument
// wasm, so construction must wait for that module to initialize — constructing
// it here would trap on the uninitialized wasm binding and take the whole
// viewer module down with it. Until the wasm is ready (or if it fails to
// load), the H.264 path skips frames visibly; MJPEG is unaffected.
let h264Registry = null;

function buildH264Registry() {
  return new H264DecoderRegistry((target, token) =>
    new H264CanvasDecoder(target, {
      log: (message) => log(message),
      isActive: () => transportSessions.isActive(token),
    }),
  );
}

/** Decodes one MJPEG payload and blits it to `target`, resizing its canvas to
 *  the frame. Session-token checked around the async decode so a frame decoded
 *  after teardown is dropped. */
async function paintJpeg(payload, target, token) {
  const bitmap = await createImageBitmap(new Blob([payload], { type: "image/jpeg" }));
  if (!transportSessions.isActive(token)) {
    bitmap.close();
    return;
  }
  const { canvas, ctx: targetCtx } = target;
  if (canvas.width !== bitmap.width || canvas.height !== bitmap.height) {
    canvas.width = bitmap.width;
    canvas.height = bitmap.height;
  }
  targetCtx.drawImage(bitmap, 0, 0);
  bitmap.close();
}

/** Routes an admitted frame's payload to the decoder for its codec, counting
 *  and logging a skip for a codec this viewer cannot decode. MJPEG paints
 *  directly; H.264 (Annex-B) routes to a per-source WebCodecs decoder, which
 *  fails closed when WebCodecs or the stream's profile is unavailable. */
async function paintByCodec(fourcc, payload, sourceId, target, token) {
  if (fourcc === FOURCC_MJPEG) {
    await paintJpeg(payload, target, token);
    return;
  }
  if (fourcc === FOURCC_H264) {
    if (!h264Registry) {
      state.skippedVideoFrames += 1;
      if (!state.h264UnavailableLogged) {
        state.h264UnavailableLogged = true;
        log("H.264 decode unavailable (instrument wasm not loaded); skipping H.264 frames silently");
      }
      return;
    }
    h264Registry.for(sourceId, target, token).decode(payload);
    return;
  }
  state.skippedVideoFrames += 1;
  log(`unknown video codec FourCC "${fourcc}" for source ${sourceId}; skipping frame (${state.skippedVideoFrames} skipped total)`);
}

/** Resolves a frame's canvas target by source id, counting/logging a skip for
 *  an unknown source. Returns the target, or `null` to skip. */
function videoTargetFor(sourceId) {
  return resolveVideoTarget(VIDEO_TARGETS, sourceId, (unknownSourceId) => {
    state.skippedVideoFrames += 1;
    log(
      `unknown video source_id ${unknownSourceId}; skipping frame ` +
        `(${state.skippedVideoFrames} skipped total)`,
    );
  });
}

// v1 video body `[source_id][fourcc][u32 LE len][payload]` (ADR-0016), retained
// so a host that has not adopted the v2 capture-identity framing still renders.
async function renderVideoFrame(body, token) {
  if (!transportSessions.isActive(token)) return;
  if (body.length < 9) return;
  const sourceId = body[0];
  const fourcc = String.fromCharCode(body[1], body[2], body[3], body[4]);
  const view = new DataView(body.buffer, body.byteOffset + 5, 4);
  const len = view.getUint32(0, true);
  const payload = body.subarray(9, 9 + len);
  if (payload.length !== len) {
    log(`video frame length mismatch: declared ${len}, got ${payload.length}`);
    return;
  }
  const target = videoTargetFor(sourceId);
  if (target) await paintByCodec(fourcc, payload, sourceId, target, token);
}

// v2 video body: a capture-identity header, then `[fourcc][u32 LE len][payload]`
// (ADR-0020). The capture identity gates the frame through `videoIdentity`: a
// duplicate, reordered, stale-epoch, or wrong-camera frame is dropped and never
// blitted, so a replayed frame cannot displace a newer one or refresh its age.
// Association runs ONLY on a frame the tracker accepted (via
// `associateIfAccepted`), so a replay can never associate fresh. It finds the
// aircraft snapshot corresponding to the frame's CAPTURE time and passes the
// recognized calibrations for the frame's camera (from the hash-verified
// artifact, ADR-0021) to the gate. The verdict is a diagnostic surface only;
// no conformal overlay is drawn. Live, the FPV calibration now resolves, but
// the verdict still stays not-ready because the Gazebo rover publishes planar
// pose rather than an avionics snapshot to associate against, and Aviate's
// video clock mapping is unavailable — honestly.
// Per-source conformal-readiness, so a persistent state (e.g. Aviate's
// mapping-unavailable) is logged once on transition instead of every frame.
// `undefined` = never logged; `null` = last logged ready; a string = last
// logged not-ready reason. Reset per session (resetVideoDiagnostics).
const videoReadinessLog = new Map();

// Coalesces uni-stream read failures: under the host's per-frame stall/reset
// regime a reset surfaces as a WebTransportError on the frame's stream, which
// after the first is expected and must not spam one line per frame.
// `lastLoggedMs === null` means never logged, so the first failure logs at once.
const streamReadFailures = { count: 0, lastLoggedMs: null };
const STREAM_READ_FAILURE_LOG_INTERVAL_MS = 2000;

function resetVideoDiagnostics() {
  videoReadinessLog.clear();
  streamReadFailures.count = 0;
  streamReadFailures.lastLoggedMs = null;
}

/** Logs a per-source conformal-readiness change once, never per frame. */
function logVideoReadiness(sourceId, association) {
  const transition = readinessTransition(
    videoReadinessLog.get(sourceId),
    association.ready,
    association.reason,
  );
  if (!transition) return;
  videoReadinessLog.set(sourceId, transition.state);
  log(`video source ${sourceId} ${transition.message}`);
}

async function renderVideoFrameV2(body, token) {
  if (!transportSessions.isActive(token)) return;
  // Decode + capture-identity contract validation happen in wasm, from the
  // host's own wire definitions; `decoded` carries the meta (u64 fields as
  // BigInt, u32 as Number — exactly the kinds the gate checks), the codec, the
  // payload's position in `body`, and a typed `fault` when the header violates
  // the encoder contract. A structurally malformed body decodes to null.
  const decoded = decodeVideoFrameV2(body);
  if (!decoded) {
    state.skippedVideoFrames += 1;
    log(`malformed v2 video frame; skipping (${state.skippedVideoFrames} skipped total)`);
    return;
  }
  const { meta, fourcc, payloadOffset, payloadLen, fault } = decoded;
  if (fault) {
    state.droppedIdentityFrames += 1;
    log(
      `video frame rejected (${fault.field}:${fault.rule}) for source ${meta.sourceId} ` +
        `seq ${meta.sequence}; dropped (${state.droppedIdentityFrames} identity drops total)`,
    );
    return;
  }
  const target = videoTargetFor(meta.sourceId);
  if (!target) return;
  const payload = body.subarray(payloadOffset, payloadOffset + payloadLen);
  // The calibration effective-window check is genuine wall-clock (a calibration
  // is valid for a real-time period), unlike the capture-time association.
  const nowUnixNs = BigInt(Date.now()) * 1_000_000n;
  const recognizedCalibrations = calibrationRegistry.recognizedFor(meta.cameraId, nowUnixNs);
  const { admit, association } = associateIfAccepted(videoIdentity, snapshotHistory, meta, {
    recognizedCalibrations,
  });
  if (!admit.accepted) {
    state.droppedIdentityFrames += 1;
    log(
      `video frame not admitted (${admit.reason}) for source ${meta.sourceId} ` +
        `seq ${meta.sequence}; dropped (${state.droppedIdentityFrames} identity drops total)`,
    );
    return;
  }
  if (admit.discontinuity) {
    log(`video source ${meta.sourceId} capture discontinuity: fresh epoch/incarnation/calibration`);
    // A discontinuity is a GOP boundary an H.264 decoder cannot span; drop this
    // source's decoder so the next keyframe reconfigures a fresh one (MJPEG: no-op).
    h264Registry?.reset(meta.sourceId);
  }
  state.lastAssociation = association;
  logVideoReadiness(meta.sourceId, association);
  await paintByCodec(fourcc, payload, meta.sourceId, target, token);
}

/** Reads telemetry-fast datagrams (bare Envelope, TelemetrySample arm) forever, updating the pose overlay. */
async function readTelemetryDatagrams(transport, token) {
  if (!transportSessions.isActive(token)) return;
  const reader = transport.datagrams.readable.getReader();
  if (!transportSessions.trackReader(token, reader)) return;
  try {
    for (;;) {
      const { value, done } = await reader.read();
      if (!transportSessions.isActive(token)) return;
      if (done) return;
      const decoded = decodeDatagramEnvelope(value);
      if (decoded.kind === "TelemetrySample") {
        const t = decoded.message;
        if (t.avionics) {
          instruments.ingress.ingest(t, performance.now());
          // Feed the association ring from the ACCEPTED snapshot the ingress
          // exposes (not the raw wire sample), anchored on the kinematics group:
          // position is what registers world features for a conformal overlay.
          // A snapshot without kinematics supplies no spatial anchor and is not
          // observed, so association stays fail-closed for it.
          const accepted = instruments.ingress.snapshot(performance.now());
          if (accepted.kinematics) {
            snapshotHistory.observe(accepted.kinematics.stamp);
          }
        }
        if (t.vehicleId !== VEHICLE_ID) continue;
        const fcView = instruments.fcState.observe(t.fcState ?? null, performance.now());
        logFcCommandVerdict(fcView);
        els.telemetry.textContent = formatTelemetrySummary(t, fcView);
      } else if (decoded.kind === "Pong") {
        // RTT probing is out of scope for this demo viewer; ignored.
      } else if (decoded.kind === "FrameRejected") {
        const r = decoded.message;
        log(
          `control frame rejected (reason ${r.reason}) scope=${r.scope} ` +
            `seq=${r.sequence} hostGen=${r.currentGeneration}`,
        );
        // A fenced rejection on the GIMBAL scope while we think we hold it:
        // the host revoked (e.g. the silence watchdog during a stall). Drop
        // the local grant so the quasimode's lease planner re-requests,
        // instead of streaming rejected frames forever.
        if (
          (r.reason === 1 || r.reason === 2) &&
          r.scope === GIMBAL_SCOPE &&
          state.gimbalLeaseGranted
        ) {
          state.gimbalLeaseGranted = false;
          log("host fenced our gimbal authority; the lease planner will re-request");
        }
        // Reasons 1 (stale generation) and 2 (no holder) on the motion
        // group while we believe we hold it mean the host fenced us out —
        // typically the silence watchdog revoked during a stall. Drop the
        // local grant and re-request (debounced), instead of streaming
        // rejected frames forever.
        if (
          (r.reason === 1 || r.reason === 2) &&
          motionGroup(r.scope) === motionGroup(state.motionScope) &&
          state.leaseGranted
        ) {
          state.leaseGranted = false;
          state.motionRecovered = false;
          if (r.currentGeneration !== undefined) {
            state.motionFence = BigInt(r.currentGeneration);
          }
          const nowMs = performance.now();
          if (nowMs - (state.lastAuthorityReacquireMs ?? 0) > 500) {
            state.lastAuthorityReacquireMs = nowMs;
            const writer = state.sessionWriter;
            if (writer) {
              const request = encodeLeaseRequestEnvelope({
                vehicleId: VEHICLE_ID,
                scope: state.motionScope,
              });
              writer.write(lengthDelimit(request)).catch(() => {});
              log("host fenced our motion authority; reacquiring the lease");
            }
          }
        }
      }
    }
  } finally {
    transportSessions.untrackReader(token, reader);
  }
}

// ---- keyboard -> control frame datagrams -----------------------------------

// Which keys the page captures is a device-profile question, answered by the
// runtime's keyboard profile data (boundKey) — no key list lives here. Keys
// are canonicalized (letters lower-cased) so the profile speaks one form.
function canonicalKey(key) {
  return key.length === 1 ? key.toLowerCase() : key;
}
function forwardKey(event, pressed) {
  if (!state.controlShell) return;
  const key = canonicalKey(event.key);
  if (!state.controlShell.boundKey(key)) return;
  state.controlShell.keyEvent(key, pressed);
  event.preventDefault();
}
window.addEventListener("keydown", (event) => forwardKey(event, true));
window.addEventListener("keyup", (event) => forwardKey(event, false));

// A pad disconnect — including the disconnect half of a same-model
// replacement — returns control to the keyboard TRANSACTIONALLY and
// re-announces the keyboard's real identity. The reconnect (or the
// replacement's connect half) re-selects through the same path, so a new
// physical unit is always a fresh activation.
window.addEventListener("gamepaddisconnected", (event) => {
  if (!state.controlShell) return;
  if (state.selectedPadId !== null && event.gamepad?.id === state.selectedPadId) {
    state.selectedPadId = null;
    state.controlShell.deselectDevice();
    log(`gamepad disconnected: ${event.gamepad.id}; control returns to the keyboard`);
  }
});

/** Returns the first connected gamepad that matches a known profile, else any
 *  connected gamepad, else null. A gamepad is exposed to the page only after the
 *  user moves a stick or presses a button once. */
function activeGamepad() {
  const pads = (navigator.getGamepads && navigator.getGamepads()) || [];
  for (const pad of pads) {
    if (pad && pad.connected) return pad;
  }
  return null;
}

/** Sends one control-fast datagram at `CONTROL_HZ`, carrying the latest key-derived axes (superseded samples are droppable, ADR-0011). */
function startControlLoop(transport, token) {
  const started = startDatagramControl({
    datagrams: transport.datagrams,
    lifecycle: transportSessions,
    token,
    run: (writer) => runControlLoop(writer, token),
    onError: (error) => log(`control loop stopped: ${error}`),
  });
  if (!started.ok) {
    const detail =
      started.reason === "send-stream-unavailable"
        ? "no datagram send stream"
        : "datagram writer acquisition failed";
    els.gamepad.textContent = `control unavailable — ${detail}`;
    els.overlay.textContent = "control unavailable";
    log(`control unavailable: ${detail}`);
  }
  return started.ok;
}

/** Runs the control-fast send loop with an acquired, session-owned writer. */
async function runControlLoop(writer, token) {
  // The control runtime owns every edge baseline (arm/disarm/R3), so a button
  // held across a reconnect cannot read as a fresh edge — no priming here.
  const intervalMs = 1000 / CONTROL_HZ;
  // Self-paced async loop rather than setInterval: it awaits the writer's
  // backpressure signal (`ready`) before each send, so datagrams never queue up
  // in the WritableStream and get flushed in a burst with stale `sampled_at`
  // (which the host rejects as too old, ADR-0009). `sampled_at` is stamped right
  // before the write, after `ready`, so it reflects the real send moment.
  while (transportSessions.isActive(token) && state.connected) {
      try {
        await writer.ready;
      } catch {
        return; // writer closed (session ended)
      }
      if (!transportSessions.isActive(token) || !state.connected) return;
      // A connected gamepad drives under its profile; the keyboard is the
      // fallback when none is present. The readout shows the live mapping.
      const mode = els.flightMode ? els.flightMode.value : "rover";
      if (!controlGate.mayPublish()) {
        // Input loss RELINQUISHES control authority. The latch is
        // absolute: NO frame — not even a neutral — is sent under this
        // generation, because the explicit LeaseRelease (sent by the
        // blur handler the instant the latch set; re-attempted here for
        // the polled-unfocused fallback) makes the host fence the
        // generation and drive the vehicle to its link-loss policy. A
        // client-side neutral would both violate the latch invariant and
        // refresh the very setpoint freshness the silence watchdog (the
        // independent backup) is watching.
        sendLeaseRelease(token);
        els.gamepad.textContent =
          "control released — input lost; press Connect to resume control";
        log("input lost — control authority released (host acknowledges or watchdog covers)");
        return;
      }
      // The control runtime maps every input; until its wasm has loaded there
      // is no mapping to run, so idle this tick rather than send anything.
      if (!state.controlShell) {
        await new Promise((resolve) => setTimeout(resolve, intervalMs));
        continue;
      }
      // One WASM call per tick maps the raw sample (gamepad or keyboard) to
      // the whole plan: the motion frame, the gimbal quasimode frame, and the
      // lease action. All device mapping, curves, masking, edge detection, and
      // lease planning live in the runtime; this shell only executes the plan.
      const pad = activeGamepad();
      // Resolve the pad's profile through the runtime's shared selector the
      // moment its identity changes. A refused selection (ambiguous registry)
      // keeps NO device map: those ticks sample empty and drive nothing.
      if (pad && pad.id !== state.selectedPadId) {
        state.selectedPadId = pad.id;
        const outcome = state.controlShell.selectDevice(pad.id);
        if (outcome === null) {
          log(`gamepad REFUSED (ambiguous device-profile registry): ${pad.id}`);
        } else {
          log(`gamepad selected (${outcome}): ${pad.id}`);
        }
      } else if (!pad && state.selectedPadId !== null) {
        // Poll-level backstop for a disconnect whose DOM event was missed.
        state.selectedPadId = null;
        state.controlShell.deselectDevice();
        log("gamepad gone; control returns to the keyboard");
      }
      const sessionState = {
        generation: state.controlGeneration,
        mode,
        connected: state.connected,
        leaseGranted: state.gimbalLeaseGranted,
        leaseDenied: state.gimbalLeaseDenied,
        // The MOTION lease grant: the runtime gates all motion output until it
        // is regranted on a fresh generation after a profile handover, so a
        // remapped scheme never publishes on the released generation.
        motionGranted: state.leaseGranted,
        // A denied reacquire is terminal — the runtime stops re-requesting.
        motionDenied: state.motionDenied,
        // The runtime stays neutralizing after a handover until the host
        // confirms it cleared the vehicle's link-loss latch on this generation.
        motionRecovered: state.motionRecovered,
        nowMs: performance.now(),
      };
      const plan = pad
        ? state.controlShell.tickFromPad(pad, sessionState)
        : state.controlShell.tickFromKeys(sessionState);
      updateControlReadout(pad, mode, plan);
      reportSuppressedPresses(plan);
      reportExpiredActions();
      if (state.pendingReset) {
        state.pendingReset = false;
        requestSimReset();
      }
      // A completed activation handover advances the revision; the
      // re-announcement must reach the host before any frame carries it.
      // The runtime emits no frames on the install tick (frames are
      // datagrams and could beat this ordered-stream write), and a
      // transfer's lease reacquisition is queued below, after this write,
      // on the same ordered stream.
      maybeAnnounceProfileActivation();
      // Re-check the latch immediately before the write: no frame may be
      // sent under this generation once input loss latched, regardless of
      // which await the latch interleaved with.
      if (!controlGate.mayPublish()) continue;
      if (plan.motion) {
        sendMotionFrame(writer, token, mode, plan);
      }
      if (plan.gimbal) {
        sendGimbalFrame(writer, token, plan.gimbal);
      }
      if (plan.lease) {
        executeLeaseAction(token, plan.lease, GIMBAL_SCOPE);
      }
      if (plan.motionLease) {
        // Only a scope-member transfer cycles the motion lease (the host
        // fences the old flight generation before the new member runs);
        // a same-scope mapping swap retains authority and emits no
        // action. Distinct from the input-loss release, which drives the
        // vehicle's link-loss policy.
        executeLeaseAction(token, plan.motionLease, state.motionScope);
      }
      await new Promise((resolve) => setTimeout(resolve, intervalMs));
  }
}

function velocityCapabilityFor(scope) {
  return capabilityFor(state.advertisedScopes, VEHICLE_ID, scope, INTENT_FAMILY_VELOCITY);
}

/** The fencing generation the client holds `scope` at — the binding every
 *  action command carries. */
function generationForScope(scope) {
  if (scope === GIMBAL_SCOPE) return state.gimbalGeneration;
  if (scope === SIM_LIFECYCLE_SCOPE) return state.lifecycle.generation;
  return state.generation;
}

/** The reset press: a LIFECYCLE action under its own lease (SIM-01), never
 *  flight authority. Leases the scope on demand, sends the reset once
 *  granted, and releases when the result lands. */
function requestSimReset() {
  if (!actionAdvertised(state.advertisedScopes, VEHICLE_ID, SIM_LIFECYCLE_SCOPE, CONTROL_ACTION.simReset)) {
    log("sim reset not advertised (not a simulator host); not sent");
    return;
  }
  if (state.lifecycle.granted) {
    if (requestAction(SIM_LIFECYCLE_SCOPE, CONTROL_ACTION.simReset)) {
      log("simulation reset requested");
    }
    return;
  }
  state.lifecycle.pendingPress = true;
  const writer = state.sessionWriter;
  if (!writer) return;
  const request = encodeLeaseRequestEnvelope({ vehicleId: VEHICLE_ID, scope: SIM_LIFECYCLE_SCOPE });
  writer.write(lengthDelimit(request)).catch(() => {});
  log(`sent LeaseRequest for ${SIM_LIFECYCLE_SCOPE} (reset authorization)`);
}

/** Gates a press on the vehicle's advertisement (CTRL-01: nothing sends
 *  without a matching capability — a suppressed press is logged so the
 *  operator sees WHY nothing fired), then sends it ONCE as a
 *  `ControlActionCommand` on the RELIABLE ordered session stream, bound to
 *  the session, vehicle, scope, held generation, and announced activation
 *  revision. The press stays pending until its result echoes the id. */
function requestAction(scope, action, modeTarget, cancels = []) {
  if (!actionAdvertised(state.advertisedScopes, VEHICLE_ID, scope, action, modeTarget)) {
    log(`action ${action} not advertised for ${scope}; not sent`);
    return false;
  }
  const writer = state.sessionWriter;
  if (!writer) {
    log(`action ${action}: no session stream; not sent`);
    return false;
  }
  const actionId = enqueueAction(state.actionTracker, scope, action, performance.now(), {
    modeTarget,
    cancels,
  });
  const command = encodeControlActionCommandEnvelope({
    sessionId: state.sessionId,
    vehicleId: VEHICLE_ID,
    scope,
    generation: generationForScope(scope),
    activationRevision: state.controlShell.activationRevision(),
    action,
    modeTarget,
    actionId,
  });
  writer.write(lengthDelimit(command)).catch(() => {
    log(`action ${action} (id ${actionId}): session stream write failed`);
  });
  return true;
}

/** The explicit per-action outcome (CTRL-01): a press is never silently
 *  dropped, a rejection is loud, and local mode state changes ONLY here —
 *  on the host's acceptance — never optimistically on send. */
function handleActionResult(m) {
  const verdict = m.accepted ? "accepted" : `REJECTED (${m.detail})`;
  const id = m.actionId ? ` id=${m.actionId}` : "";
  log(`action result [${m.scope} seq=${m.sequence}] action=${m.action}${id} ${verdict}`);
  if (!m.actionId) return;
  const entry = resolveAction(state.actionTracker, m.actionId);
  if (!entry) return; // a replay of an already-settled press
  if (entry.scope === SIM_LIFECYCLE_SCOPE) {
    // The lifecycle lease exists only for this press: release it so the
    // silence watchdog never has a held-but-frameless scope to revoke.
    state.lifecycle.granted = false;
    const writer = state.sessionWriter;
    if (writer) {
      const release = encodeLeaseReleaseEnvelope({
        vehicleId: VEHICLE_ID,
        scope: SIM_LIFECYCLE_SCOPE,
      });
      writer.write(lengthDelimit(release)).catch(() => {});
      log(`released the ${SIM_LIFECYCLE_SCOPE} lease`);
    }
  }
  if (entry.action === CONTROL_ACTION.modeRequest && m.accepted) {
    state.fpvActive = entry.modeTarget === MODE_TARGET.fpvDirect;
    els.overlay.textContent = state.fpvActive ? "FPV mode engaged" : "camera mode engaged";
    log(`mode ack: ${state.fpvActive ? "fpv-direct" : "camera-velocity"} engaged`);
  }
}

/** Reports every press whose answer window expired — on the reliable
 *  channel that means the transport died or the host never answered. */
function reportExpiredActions() {
  for (const gone of expirePending(state.actionTracker, performance.now())) {
    log(
      `action ${gone.action} (id ${gone.actionId}) got no result within ` +
        `${ACTION_TIMEOUT_MS} ms; the session stream may be dead`,
    );
  }
}

/** Announces the active control profile on the reliable session stream:
 *  scheme identity, document revision, monotonic activation revision, and
 *  the content digests of BOTH the scheme and the selected device profile —
 *  the composite traceability record the host validates every typed frame
 *  against (INPUT-01). Ordered BEFORE the LeaseRequest on the same stream,
 *  so the host records the activation before any frame can be accepted. */
function announceProfileActivation(writer = state.sessionWriter) {
  if (!writer || !state.controlShell) return;
  const shell = state.controlShell;
  const activation = encodeProfileActivationEnvelope({
    sessionId: state.sessionId,
    profileId: shell.profileId(),
    profileRevision: shell.profileRevision(),
    activationRevision: shell.activationRevision(),
    digest: shell.profileDigestBytes(),
    deviceProfileId: shell.deviceLabel(),
    deviceProfileRevision: shell.deviceRevision(),
    deviceDigest: shell.deviceDigestBytes(),
  });
  writer.write(lengthDelimit(activation)).catch(() => {});
  state.announcedActivationRevision = shell.activationRevision();
  const device = shell.deviceLabel() || "(keyboard only)";
  log(
    `announced profile ${shell.profileId()} rev=${shell.profileRevision()} ` +
      `activation=${shell.activationRevision()} digest=${shell.profileDigest().slice(0, 16)}... ` +
      `device=${device}`,
  );
}

/** Re-announces when the activation revision advanced past the announced
 *  one (a device swap or profile install completed its handover), so the
 *  host learns the new composite mapping before frames carry its revision —
 *  the swap's lease reacquisition rides the same ordered stream AFTER this
 *  write, and frames only flow once that lease is regranted. */
function maybeAnnounceProfileActivation() {
  if (!state.sessionWriter || !state.controlShell) return;
  if (state.controlShell.activationRevision() === state.announcedActivationRevision) return;
  announceProfileActivation();
}

/** Encodes and sends the runtime's motion frame as a TYPED velocity intent:
 *  the plan's normalized demands scale by the scope's ADVERTISED envelope
 *  (CTRL-01), so full stick commands exactly what the vehicle advertises.
 *  The runtime supplies typed arm/disarm; the DOM-driven mode-request and
 *  sim-reset actions are appended here. Fails closed (no frame) when the
 *  vehicle does not advertise a velocity intent. */
function sendMotionFrame(writer, token, mode, plan) {
  const direct = state.motionScope === DIRECT_SCOPE;
  const capability = direct
    ? capabilityFor(state.advertisedScopes, VEHICLE_ID, DIRECT_SCOPE, INTENT_FAMILY_ATTITUDE_THRUST)
    : velocityCapabilityFor(state.motionScope);
  if (!capability) return;
  const m = plan.motion;
  if (plan.arm && requestAction(state.motionScope, CONTROL_ACTION.arm, undefined, [CONTROL_ACTION.disarm])) {
    els.overlay.textContent = "ARM sent";
    log("arm command sent");
  }
  if (plan.disarm && requestAction(state.motionScope, CONTROL_ACTION.disarm, undefined, [CONTROL_ACTION.arm])) {
    els.overlay.textContent = "DISARM sent";
    log("disarm command sent");
  }
  if (state.pendingFpvToggle) {
    state.pendingFpvToggle = false;
    requestFlightModeSwitch();
  }

  let velocity;
  let attitudeThrust;
  if (direct) {
    // The client OWNS the direct-flight heading: the yaw stick slews the
    // integrated setpoint at the ADVERTISED rate, and tilt scales by the
    // advertised bound — the vehicle executes exactly this attitude,
    // reinterpreting nothing.
    const nowMs = performance.now();
    const dt = state.lastDirectFrameMs
      ? Math.min((nowMs - state.lastDirectFrameMs) / 1000, 0.1)
      : 0;
    state.lastDirectFrameMs = nowMs;
    state.fpvHeading = integrateHeading(state.fpvHeading, m.yaw, capability, dt);
    attitudeThrust = buildAttitudeThrustIntent(m, state.fpvHeading, capability);
  } else {
    velocity = buildVelocityIntent(m, mode, capability);
  }
  state.sequence = (state.sequence + 1) >>> 0; // wraps at u32, matching the wire SequenceNum width.
  const envelope = encodeControlFrameEnvelope({
    sessionId: state.sessionId,
    vehicleId: VEHICLE_ID,
    scope: state.motionScope,
    generation: state.generation,
    sequence: state.sequence,
    sampledAtNanos: nowNanos(),
    profileRevision: state.controlShell.profileRevision(),
    activationRevision: state.controlShell.activationRevision(),
    velocity,
    attitudeThrust,
  });
  writer.write(envelope).catch((error) => {
    transportSessions.runIfActive(token, () => log(`control datagram send failed: ${error}`));
  });
}

/** The latest declared sim heading (rad, local NED), or null without an
 *  attitude estimate — the direct-flight heading integrator's seed. */
function currentTelemetryHeading() {
  const snapshot = instruments.ingress?.snapshot(performance.now());
  const heading = declaredSimHeading(snapshot?.attitude);
  return heading === null ? null : heading.rad;
}

/** The FPV/camera flight toggle. When the vehicle advertises the
 *  direct-flight scope (AttitudeThrust under vehicle.motion.direct), the
 *  switch is a SCOPE HANDOVER — the runtime's transactional reactivation
 *  neutral-fences the sticks, releases the current motion lease, and the
 *  reacquisition targets the other scope under a fresh generation. The
 *  typed ModeRequest remains only for vehicles that advertise one; with
 *  neither advertised the press is suppressed loudly. */
function requestFlightModeSwitch() {
  const target = state.motionScope === DIRECT_SCOPE ? MOTION_SCOPE : DIRECT_SCOPE;
  const family =
    target === DIRECT_SCOPE ? INTENT_FAMILY_ATTITUDE_THRUST : INTENT_FAMILY_VELOCITY;
  if (capabilityFor(state.advertisedScopes, VEHICLE_ID, target, family)) {
    if (state.pendingMotionScope) return; // a handover is already in flight
    if (target === DIRECT_SCOPE && currentTelemetryHeading() === null) {
      log("no heading telemetry yet; cannot enter direct flight");
      return;
    }
    state.pendingMotionScope = target;
    state.controlShell.reactivate();
    log(`flight-scope handover to ${target}: neutral fence + lease cycle opened`);
    return;
  }
  const pendingTarget = pendingModeTarget(
    state.actionTracker,
    state.motionScope,
    CONTROL_ACTION.modeRequest,
  );
  const fpvBase =
    pendingTarget !== undefined ? pendingTarget === MODE_TARGET.fpvDirect : state.fpvActive;
  const modeTarget = fpvBase ? MODE_TARGET.cameraVelocity : MODE_TARGET.fpvDirect;
  if (
    requestAction(state.motionScope, CONTROL_ACTION.modeRequest, modeTarget, [
      CONTROL_ACTION.modeRequest,
    ])
  ) {
    log(
      `mode request sent: target=${modeTarget === MODE_TARGET.fpvDirect ? "fpv-direct" : "camera-velocity"}`,
    );
  }
}

/** Encodes and sends the runtime's gimbal frame. The runtime emits one every
 *  tick while the lease is held (zero rates when idle), so the continuous
 *  stream is the scope's liveness (ADR-0011); R3 rides as the recenter edge. */
function sendGimbalFrame(writer, token, gimbal) {
  const capability = capabilityFor(
    state.advertisedScopes,
    VEHICLE_ID,
    GIMBAL_SCOPE,
    INTENT_FAMILY_GIMBAL_RATE,
  );
  if (!capability) return;
  if (
    gimbal.recenter &&
    requestAction(GIMBAL_SCOPE, CONTROL_ACTION.gimbalRecenter)
  ) {
    log("gimbal recenter requested (R3)");
  }
  state.gimbalSequence = (state.gimbalSequence + 1) >>> 0;
  const envelope = encodeControlFrameEnvelope({
    sessionId: state.sessionId,
    vehicleId: VEHICLE_ID,
    scope: GIMBAL_SCOPE,
    generation: state.gimbalGeneration,
    sequence: state.gimbalSequence,
    sampledAtNanos: nowNanos(),
    profileRevision: state.controlShell.profileRevision(),
    activationRevision: state.controlShell.activationRevision(),
    // The plan's normalized LOS rates scale by the ADVERTISED angular
    // envelope, so full deflection slews exactly what the vehicle allows.
    gimbalRate: buildGimbalRateIntent(gimbal, capability),
  });
  writer.write(envelope).catch((error) => {
    transportSessions.runIfActive(token, () => log(`gimbal datagram send failed: ${error}`));
  });
}

/** Executes the runtime's gimbal-lease decision on the reliable session
 *  stream: request or release the `vehicle.gimbal` scope. The runtime owns the
 *  request debounce and the mode/grant/deny policy; this only sends. */
function executeLeaseAction(token, action, scope) {
  const writer = state.sessionWriter;
  if (!writer || !transportSessions.isActive(token)) return;
  if (action === "release") {
    if (scope === GIMBAL_SCOPE) state.gimbalLeaseGranted = false;
    const release = encodeLeaseReleaseEnvelope({ vehicleId: VEHICLE_ID, scope });
    writer.write(lengthDelimit(release)).catch(() => {});
    log(`released the ${scope} lease`);
  } else if (action === "request") {
    const request = encodeLeaseRequestEnvelope({ vehicleId: VEHICLE_ID, scope });
    writer.write(lengthDelimit(request)).catch(() => {});
    log(`sent LeaseRequest for ${scope}`);
  }
}

/** Shows the live control mapping in the readout: the gimbal quasimode when a
 *  gimbal frame streams, otherwise the flight axes. Display only. */
function updateControlReadout(pad, mode, plan) {
  // The selected-profile label comes from the runtime (visible identity,
  // ADR-0007); an empty label means the pad's selection was refused.
  const src = pad
    ? state.controlShell.deviceLabel() || "pad refused (ambiguous profiles)"
    : "keyboard (WS=climb AD=yaw arrows=move)";
  // Capture is shown whenever the modifier is held, even at a centered stick,
  // so #167's deliberate LT-descend suppression is always visible.
  if (plan.captureActive) {
    els.gamepad.textContent =
      `GIMBAL (LT held): pitch=${(plan.gimbal?.pitch ?? 0).toFixed(2)} yaw=${(plan.gimbal?.yaw ?? 0).toFixed(2)} | ` +
      "right stick captured, LT-descend inhibited; R3 recenters";
    return;
  }
  const m = plan.motion ?? { roll: 0, pitch: 0, throttle: 0, yaw: 0 };
  // Three honest states: "gated" (no frames — presses are suppressed),
  // "recovering" (neutral recovery frames flow but live authority has not
  // been confirmed, so presses are still suppressed), "streaming" (live).
  // The distinction keeps the pre-press indicator from overstating
  // authority during the post-revocation neutral-activation burst.
  const motionState = !plan.motion
    ? "gated"
    : state.motionRecovered
      ? "streaming"
      : "recovering";
  els.gamepad.textContent =
    `flight [${mode}]: ${src} | roll=${m.roll.toFixed(2)} pitch=${m.pitch.toFixed(2)} ` +
    `climb=${m.throttle.toFixed(2)} yaw=${m.yaw.toFixed(2)} | motion: ${motionState} | ` +
    "arm: Options/Enter disarm: Create/Backspace";
}

/** Logs each NEW FC arm/disarm verdict once (enactment truth): a refusal
 *  is loud on the overlay and log; an acceptance just clears the memory,
 *  since the arm-state readout already reflects success. */
function logFcCommandVerdict(fcView) {
  const verdict = fcView?.lastCommand ?? null;
  const key = verdict ? `${verdict.arm ? "arm" : "disarm"}:${verdict.result}` : null;
  if (key === state.lastFcVerdictLogged) return;
  state.lastFcVerdictLogged = key;
  if (verdict && verdict.result !== 0) {
    const which = verdict.arm ? "arm" : "disarm";
    els.overlay.textContent = `FC refused ${which} (result ${verdict.result})`;
    log(`FC refused the ${which} command (MAV_RESULT ${verdict.result})`);
  }
}

/** The reason a gated tick refuses arm/disarm presses, for the operator. */
function motionGateReason() {
  if (state.motionDenied) return "motion lease denied; reconnect to retry";
  if (!state.leaseGranted) return "no motion lease; press Connect to take control";
  return "motion authority recovering";
}

/** Surfaces every runtime-suppressed arm/disarm press (CTRL-01 feedback,
 *  #180): the press was consumed while motion output was gated, and a
 *  swallowed safety press must never look like a dead key. */
function reportSuppressedPresses(plan) {
  for (const [suppressed, press] of [
    [plan.armSuppressed, "arm"],
    [plan.disarmSuppressed, "disarm"],
  ]) {
    if (!suppressed) continue;
    const reason = motionGateReason();
    els.overlay.textContent = `${press} press suppressed — ${reason}`;
    log(`${press} press suppressed: ${reason}`);
  }
}

// ---- instrument panels (ADR-0017) -------------------------------------------

// DYN-01: turn-rate derivation is measurement-coherent — it advances
// only on a NEW accepted heading measurement identity and differences
// over the acquisition clock (see turn-derivation.js). Rendering
// cadence cannot inflate or zero the rate, and session retirement
// resets it below.
const turnDerivation = new TurnDerivation();

/** The explicit SIM heading declaration: local-NED yaw from the
 * published attitude quaternion, declared sim-local-true. Returns null
 * when no attitude estimate exists — no sample, never a zero heading. */
function declaredSimHeading(attitude) {
  const q = attitude?.quat;
  if (!q || ![q.w, q.x, q.y, q.z].every(Number.isFinite)) return null;
  const yaw = Math.atan2(2 * (q.w * q.z + q.x * q.y), 1 - 2 * (q.y * q.y + q.z * q.z));
  return { rad: yaw, reference: 2, ageMs: attitude.ageMs };
}


/** Maps the latest wire avionics estimate into the instrument state ABI
 * and draws both panels; runs on the display's own rAF cadence. Every
 * render result is honored: a validated frame is blitted, anything else
 * is covered by the failure page (no ignored results and no stale
 * imagery). */
function renderInstruments() {
  const mod = instruments.mod;
  if (!mod) {
    // Load/ABI/init faults are shared-backend failures: both panels show
    // the failure page. While still loading there is no prior imagery to
    // cover, so the canvases stay blank.
    if (instruments.moduleFault !== null) {
      coverInstrumentFailures(instruments.health, instrumentTargets());
    }
    return;
  }
  const snapshot = instruments.ingress.snapshot(performance.now());
  const attitude = snapshot.attitude;
  const kinematics = snapshot.kinematics;
  const validFlags = snapshot.validFlags;
  const coherence = {
    insufficient: 0,
    coherent: 1,
    "excessive-skew": 2,
  }[snapshot.coherence.status];
  // NAV-01: the feeder — not the display — declares the simulator's
  // heading explicitly. Local-NED yaw is computed HERE from the same
  // estimate the vehicle published and declared as sim-local-true
  // (reference 2); the display never derives heading from attitude on
  // its own, so removing this declaration flags HDG instead of
  // freezing a fabricated rose.
  const heading = declaredSimHeading(attitude);
  const dynamics = turnDerivation.update(
    heading === null ? NaN : heading.rad,
    heading === null ? NaN : heading.ageMs,
    attitude?.stamp ?? null,
  );
  const panelState = {
    attitude,
    kinematics,
    air: null, // no airspeed/baro sensor on Aviate's wire yet (ADR-0018): honest Missing.
    nav: null,
    wind: null,
    selections: { headingBugRad: 0, headingBugReference: 2 },
    heading,
    dynamics,
    quality: snapshot.quality,
    valid: {
      attitude: !!(validFlags & 1),
      rates: !!(validFlags & 2),
      position: !!(validFlags & 4),
      velocity: !!(validFlags & 8),
      heading: heading !== null && !!(validFlags & 1),
      turn: dynamics !== null && !!(validFlags & 1),
    },
    snapshot: { generation: snapshot.generation, coherence },
  };
  const nowMs = performance.now();
  renderInstrumentSet(mod, instruments.health, instrumentTargets(), panelState, nowMs);
}

/** Liveness check on its own timer so a stalled render loop still gets
 * its stale frame covered; skipped while the module is absent (a load
 * fault already shows its own page, and before load there is no imagery
 * to cover). */
function watchdogTick() {
  if (!instruments.mod) return;
  tickInstrumentSet(instruments.health, instrumentTargets(), performance.now());
}

async function startInstruments() {
  startDisplayLoop(
    (callback) => requestAnimationFrame(callback),
    () => renderInstruments(),
    () =>
      failInstrumentSet(
        instruments.health,
        instrumentTargets(),
        performance.now(),
        REASON.RENDER_TRAP,
      ),
  );
  try {
    // Revalidate the wasm on every load: this page is served statically with
    // no build step, so a rebuilt binary must not be masked by a heuristically
    // cached copy (a 304 keeps it cheap when unchanged).
    const wasmSource = await fetch("./instrument-runtime_bg.wasm", { cache: "no-cache" });
    instruments.mod = await loadInstruments(wasmSource);
    h264Registry = buildH264Registry();
    const nowMs = performance.now();
    for (const health of Object.values(instruments.health)) health.reset(nowMs);
    setInterval(watchdogTick, WATCHDOG_INTERVAL_MS);
    log("instrument panels ready (wasm loaded)");
  } catch (error) {
    instruments.moduleFault = error?.reason ?? REASON.WASM_LOAD;
    failInstrumentSet(
      instruments.health,
      instrumentTargets(),
      performance.now(),
      instruments.moduleFault,
    );
    log(`instrument panels unavailable (D-${instruments.moduleFault}): ${error} (run scripts/build-web-instruments.sh)`);
  }
}

/** Loads the control-runtime wasm and bootstraps it: compile the built-in
 *  default profile bytes and activate them through the normal path. The
 *  control loop stays idle until this resolves; a failure degrades to
 *  no-control (telemetry/video still run) rather than taking the page down. */
async function startControl() {
  try {
    const wasmSource = await fetch("./control-runtime_bg.wasm", { cache: "no-cache" });
    state.controlShell = await loadControlShell(wasmSource);
    log(`control runtime ready (profile revision ${state.controlShell.activationRevision()})`);
  } catch (error) {
    log(`control runtime unavailable: ${error} (run scripts/build-web-instruments.sh)`);
  }
}

window.addEventListener("pagehide", () => instruments.mod?.dispose(), { once: true });
applyUrlParams();
const instrumentsStarted = startInstruments();
startControl();
document.getElementById("fpvBtn").addEventListener("click", () => {
  state.pendingFpvToggle = true;
});
document.getElementById("resetBtn").addEventListener("click", () => {
  state.pendingReset = true;
});
els.connectBtn.addEventListener("click", () => {
  // An explicit, user-initiated start: resets the reconnect backoff and is the
  // only path that requests control.
  reconnect.requestConnect();
});
// A launcher-pinned URL (host + port + cert all present, autoconnect=1)
// connects through the SAME explicit path as the button — the URL already
// states everything the click would add. Anything less than a fully
// pinned URL still requires the click. Runs after all wiring so the full
// connect stack (control gate, reconnect controller) is in place.
//
// Deferred to VISIBILITY: a launcher-opened tab can load hidden behind
// other windows, where throttled timers starve the 30 Hz control loop
// into watchdog revoke/regrant churn and an unfocused lease probe would
// silently skip motion authority. The instrument wasm is awaited first
// so the panels are ready when the first telemetry arrives (a failed
// wasm load still connects — telemetry and video degrade visibly).
{
  const params = new URLSearchParams(window.location.search);
  if (
    params.get("autoconnect") === "1" &&
    els.host.value &&
    els.port.value &&
    els.certHash.value
  ) {
    whenVisible(document, () => {
      instrumentsStarted.finally(() => reconnect.requestConnect());
    });
  }
}

// Returning to a tab the browser had frozen is the moment to recover: the
// session likely idle-dropped while away, and only now can a reconnect succeed
// (a hidden tab would just drop again).
document.addEventListener("visibilitychange", () => {
  if (document.visibilityState === "visible") reconnect.notifyVisible();
});

// Losing window focus can swallow keyup events, leaving keys "stuck". Clear the
// keyboard and arm-edge state immediately on blur so input released off-window
// does not linger and a button still held when focus returns cannot edge.
window.addEventListener("blur", () => {
  // Latch FIRST, synchronously: the control loop may be parked on an
  // await, and a refocus before its next tick must not un-happen the
  // loss. Frames under the current generation end here — and the
  // explicit release starts HERE too, not at the loop's next tick, so a
  // Connect click racing the blur always finds the release in flight.
  controlGate.latchInputLoss();
  if (state.leaseGranted && transportSessions.active !== null) {
    sendLeaseRelease(transportSessions.active);
  }
  state.controlShell?.clearKeys();
});
