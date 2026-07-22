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
import { fcArmToken, formatTelemetrySummary, setTelemetrySessionState } from "./telemetry-display.js";
import { AvionicsIngress, FcStateTracker, INCARNATION_POLICY } from "./telemetry-ingress.js";
import { SnapshotAssociator, associateIfAccepted } from "./snapshot-association.js";
import { CalibrationRegistry, loadCalibrationRegistry } from "./calibration.js";
import { readinessTransition, shouldLogReadFailure } from "./video-diagnostics.js";
import { bindVideoTargets, resolveVideoTarget } from "./video-routing.js";
import { runIncomingStreamAcceptLoop } from "./uni-stream-accept.js";
import { readUniStream } from "./uni-stream.js";
import {
  decodeLengthDelimitedEnvelope,
  STREAM_KIND_AUTHORITY,
  STREAM_KIND_VIDEO,
  STREAM_KIND_VIDEO_V2,
} from "./wire.js";

const WATCHDOG_INTERVAL_MS = 250;
const FOURCC_MJPEG = "MJPG";
const STREAM_READ_FAILURE_LOG_INTERVAL_MS = 2000;

/** Builds cockpit rendering, telemetry presentation, and video readout. */
export function createCockpitReadout({
  state,
  els,
  transportSessions,
  vehicleId,
  instrumentSourceId,
  coherenceLimitNanos,
  authorityFor,
  controlGate,
  dispatchAuthorityEnvelope,
  handleFrameRejected,
  handleTransportClosed,
}) {
  const pfdCtx = els.pfd.getContext("2d");
  const hsiCtx = els.hsi.getContext("2d");
  const pfdFaultPresenter = createDomFaultPresenter(els.pfd);
  const hsiFaultPresenter = createDomFaultPresenter(els.hsi);
  const videoIdentity = new VideoIdentityTracker();
  const snapshotHistory = new SnapshotAssociator();
  const turnDerivation = new TurnDerivation();
  const videoTargets = bindVideoTargets(document);
  const videoReadinessLog = new Map();
  const streamReadFailures = { count: 0, lastLoggedMs: null };
  let h264Registry = null;
  let calibrationRegistry = new CalibrationRegistry();

  loadCalibrationRegistry("./sim-fpv-calibration.json")
    .then((registry) => {
      calibrationRegistry = registry;
    })
    .catch(() => {});

  function newSimulatorAvionicsIngress() {
    return new AvionicsIngress({
      vehicleId,
      sourceId: instrumentSourceId,
      incarnationPolicy: INCARNATION_POLICY.SIM_ACCEPT_UNSEEN,
      maximumSkewNanos: coherenceLimitNanos,
    });
  }

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

  const surface = {
    armAccepted() {
      els.overlay.textContent = "arm sequence accepted — motion uplink active";
    },
    authorityNotice(arm) {
      els.overlay.textContent = `authority: ${arm}`;
    },
    commandSent(arm) {
      els.overlay.textContent = arm ? "ARM sent" : "DISARM sent";
    },
    controlReleased() {
      els.gamepad.textContent = "control released — input lost; press Resume control";
    },
    controlResumed() {
      els.overlay.textContent = "control resumed — center inputs for neutral recovery";
    },
    controlSuspended() {
      els.overlay.textContent = "control suspended — press Resume control";
    },
    controlUnavailable(detail) {
      els.gamepad.textContent = `control unavailable — ${detail}`;
      els.overlay.textContent = "control unavailable";
    },
    gimbalDenied() {
      els.overlay.textContent = "gimbal lease denied; flight control is unaffected";
    },
    inputLostDuringConnect() {
      els.gamepad.textContent =
        "control released — input lost during connect; press Connect to resume";
    },
    leaseDenied(reason) {
      els.overlay.textContent = `lease denied (reason ${reason})`;
    },
    lifecycleDenied() {
      els.overlay.textContent = "simulation reset authorization denied";
    },
    modeEngaged(fpv) {
      els.overlay.textContent = fpv ? "FPV mode engaged" : "camera mode engaged";
    },
    motionDenied() {
      els.overlay.textContent = "motion lease denied; control stays suspended";
    },
    motionScopeGranted(fpv) {
      els.overlay.textContent = fpv
        ? "direct (FPV) flight scope granted"
        : "camera flight scope granted";
    },
    noControlLease(manual) {
      els.overlay.textContent = manual
        ? "no control lease — telemetry/video only; press Connect to take control"
        : "reconnected without control; press Connect to resume";
    },
    resumeDenied() {
      els.overlay.textContent = "same-session resume denied — press Connect to retry";
    },
    resumeResult(interrupted) {
      els.overlay.textContent = interrupted
        ? "resume interrupted by input loss"
        : "resume requested — waiting for fresh motion authority";
    },
    uplinkIdle() {
      els.overlay.textContent = "motion uplink idle — press arm to start control";
    },
  };

  function retireSessionPresentation(phase) {
    instruments.ingress = newSimulatorAvionicsIngress();
    instruments.fcState = new FcStateTracker();
    snapshotHistory.reset();
    turnDerivation.reset();
    h264Registry?.closeAll();
    setTelemetrySessionState(els, phase);
  }

  function beginTelemetrySession() {
    instruments.ingress = newSimulatorAvionicsIngress();
    instruments.fcState = new FcStateTracker();
    setTelemetrySessionState(els, "awaiting");
  }

  function noteStreamReadFailure(error) {
    streamReadFailures.count += 1;
    const nowMs = performance.now();
    if (
      !shouldLogReadFailure(
        nowMs,
        streamReadFailures.lastLoggedMs,
        STREAM_READ_FAILURE_LOG_INTERVAL_MS,
      )
    ) return;
    streamReadFailures.lastLoggedMs = nowMs;
    log(`uni stream read failed: ${error} (${streamReadFailures.count} total this session)`);
  }

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

  function buildH264Registry() {
    return new H264DecoderRegistry((target, token) =>
      new H264CanvasDecoder(target, {
        log: (message) => log(message),
        isActive: () => transportSessions.isActive(token),
      }),
    );
  }

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

  function videoTargetFor(sourceId) {
    return resolveVideoTarget(videoTargets, sourceId, (unknownSourceId) => {
      state.skippedVideoFrames += 1;
      log(
        `unknown video source_id ${unknownSourceId}; skipping frame ` +
          `(${state.skippedVideoFrames} skipped total)`,
      );
    });
  }

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

  function resetVideoDiagnostics() {
    videoReadinessLog.clear();
    streamReadFailures.count = 0;
    streamReadFailures.lastLoggedMs = null;
  }

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
      h264Registry?.reset(meta.sourceId);
    }
    state.lastAssociation = association;
    logVideoReadiness(meta.sourceId, association);
    await paintByCodec(fourcc, payload, meta.sourceId, target, token);
  }

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
          const telemetry = decoded.message;
          if (telemetry.avionics) {
            instruments.ingress.ingest(telemetry, performance.now());
            const accepted = instruments.ingress.snapshot(performance.now());
            if (accepted.kinematics) snapshotHistory.observe(accepted.kinematics.stamp);
          }
          if (telemetry.vehicleId !== vehicleId) continue;
          const fcView = instruments.fcState.observe(telemetry.fcState ?? null, performance.now());
          state.lastFcView = fcView;
          logFcCommandVerdict(fcView);
          els.telemetry.textContent = formatTelemetrySummary(telemetry, fcView);
        } else if (decoded.kind === "FrameRejected") {
          handleFrameRejected(decoded.message);
        }
      }
    } finally {
      transportSessions.untrackReader(token, reader);
    }
  }

  function updateControlReadout(pad, mode, plan) {
    const src = pad
      ? state.controlShell.deviceLabel() || "pad refused (ambiguous profiles)"
      : "keyboard (WS=climb AD=yaw arrows=move)";
    if (plan.captureActive) {
      els.gamepad.textContent =
        `GIMBAL (LT held): pitch=${(plan.gimbal?.pitch ?? 0).toFixed(2)} yaw=${(plan.gimbal?.yaw ?? 0).toFixed(2)} | ` +
        "right stick captured, LT-descend inhibited; R3 recenters";
      return;
    }
    const motion = plan.motion ?? { roll: 0, pitch: 0, throttle: 0, yaw: 0 };
    const motionAuthority = authorityFor(state.motionScope);
    const motionState = motionAuthority.needsArm
      ? "needs arm"
      : !plan.motion ? "gated" : motionAuthority.recovered ? "streaming" : "recovering";
    const armHint = state.controlShell.armHint() || "?";
    const disarmHint = state.controlShell.disarmHint() || "?";
    els.gamepad.textContent =
      `flight [${mode}]: ${src} | roll=${motion.roll.toFixed(2)} pitch=${motion.pitch.toFixed(2)} ` +
      `climb=${motion.throttle.toFixed(2)} yaw=${motion.yaw.toFixed(2)} | motion: ${motionState} | ` +
      `${fcArmToken(state.lastFcView)} | arm: ${armHint} disarm: ${disarmHint}`;
  }

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

  function motionGateReason() {
    if (controlGate.isLatched()) return "control suspended — press Resume control";
    const authority = authorityFor(state.motionScope);
    if (authority.denied) return "motion lease denied; reconnect to retry";
    if (!authority.granted) return "no motion lease; press Connect to take control";
    return "motion authority recovering";
  }

  function reportSuppressedPresses(plan) {
    for (const [suppressed, press] of [
      [plan.armSuppressed || (controlGate.isLatched() && plan.arm), "arm"],
      [plan.disarmSuppressed || (controlGate.isLatched() && plan.disarm), "disarm"],
    ]) {
      if (!suppressed) continue;
      const reason = motionGateReason();
      els.overlay.textContent = `${press} press suppressed — ${reason}`;
      log(`${press} press suppressed: ${reason}`);
    }
  }

  function declaredSimHeading(attitude) {
    const quaternion = attitude?.quat;
    if (!quaternion || ![quaternion.w, quaternion.x, quaternion.y, quaternion.z].every(Number.isFinite)) {
      return null;
    }
    const yaw = Math.atan2(
      2 * (quaternion.w * quaternion.z + quaternion.x * quaternion.y),
      1 - 2 * (quaternion.y * quaternion.y + quaternion.z * quaternion.z),
    );
    return { rad: yaw, reference: 2, ageMs: attitude.ageMs };
  }

  function currentTelemetryHeading() {
    const snapshot = instruments.ingress?.snapshot(performance.now());
    const heading = declaredSimHeading(snapshot?.attitude);
    return heading === null ? null : heading.rad;
  }

  function renderInstruments() {
    const mod = instruments.mod;
    if (!mod) {
      if (instruments.moduleFault !== null) {
        coverInstrumentFailures(instruments.health, instrumentTargets());
      }
      return;
    }
    const snapshot = instruments.ingress.snapshot(performance.now());
    const attitude = snapshot.attitude;
    const kinematics = snapshot.kinematics;
    const validFlags = snapshot.validFlags;
    const coherence = { insufficient: 0, coherent: 1, "excessive-skew": 2 }[
      snapshot.coherence.status
    ];
    const heading = declaredSimHeading(attitude);
    const dynamics = turnDerivation.update(
      heading === null ? NaN : heading.rad,
      heading === null ? NaN : heading.ageMs,
      attitude?.stamp ?? null,
    );
    const panelState = {
      attitude,
      kinematics,
      air: null,
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
    renderInstrumentSet(
      mod,
      instruments.health,
      instrumentTargets(),
      panelState,
      performance.now(),
    );
  }

  function watchdogTick() {
    if (!instruments.mod) return;
    tickInstrumentSet(instruments.health, instrumentTargets(), performance.now());
  }

  async function startInstruments() {
    startDisplayLoop(
      (callback) => requestAnimationFrame(callback),
      () => renderInstruments(),
      () => failInstrumentSet(
        instruments.health,
        instrumentTargets(),
        performance.now(),
        REASON.RENDER_TRAP,
      ),
    );
    try {
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

  return {
    acceptIncomingUniStreams,
    beginTelemetrySession,
    currentTelemetryHeading,
    dispose: () => instruments.mod?.dispose(),
    log,
    readTelemetryDatagrams,
    reportSuppressedPresses,
    resetVideoDiagnostics,
    retireSessionPresentation,
    startInstruments,
    surface,
    updateControlReadout,
  };
}
