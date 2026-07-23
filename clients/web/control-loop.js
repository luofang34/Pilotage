import {
  CONTROL_ACTION,
  MODE_TARGET,
  decodeLengthDelimitedEnvelope,
  encodeControlActionCommandEnvelope,
  encodeControlFrameEnvelope,
} from "./wire.js";
import {
  ACTION_TIMEOUT_MS,
  enqueueAction,
  expirePending,
  pendingModeTarget,
  resolveAction,
} from "./action-tracker.js";
import { createDatagramRunStop, startDatagramControl } from "./datagram-control.js";
import { resumeGrantDecision, resumeRefusalReason, resumeSessionControl } from "./resume-control.js";
import { loadControlShell } from "./control-shell.js";
import { writeLeaseAction } from "./lease-executor.js";
import { applyAuthorityTransition } from "./authority-transition.js";
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

/** Builds live control, suspended input watching, and lease execution. */
export function createControlLoop({
  state,
  els,
  transportSessions,
  controlGate,
  releaseTracker,
  vehicleId,
  motionScope,
  directScope,
  gimbalScope,
  lifecycleScope,
  frameRejectionUplinkIdle,
  controlHz,
  log,
  surface,
  updateControlReadout,
  reportSuppressedPresses,
  currentTelemetryHeading,
  lengthDelimit,
  maybeAnnounceProfileActivation,
  requestReconnect,
}) {
  function motionGroup(scope) {
    return scope === directScope ? motionScope : scope;
  }

  function authoritySlot(scope) {
    if (scope === gimbalScope) return "gimbal";
    if (scope === lifecycleScope) return "lifecycle";
    return "motion";
  }

  function authorityFor(scope) {
    return (
      state.controlShell?.authority(authoritySlot(scope)) ?? {
        generation: 0n,
        granted: false,
        denied: false,
        recovered: false,
        needsArm: false,
      }
    );
  }

  function nowNanos() {
    return state.startNanos + BigInt(Math.round(performance.now() * 1_000_000));
  }

  function appendBytes(existing, incoming) {
    const out = new Uint8Array(existing.length + incoming.length);
    out.set(existing, 0);
    out.set(incoming, existing.length);
    return out;
  }

  function executeLeaseAction(token, action, scope, writer = state.sessionWriter) {
    if (!writer || !token || !transportSessions.isActive(token)) {
      return Promise.resolve(false);
    }
    const write = writeLeaseAction({ writer, action, vehicleId, scope, frame: lengthDelimit });
    if (!write) return Promise.resolve(false);
    const verb = action === "release" ? "released" : "requested";
    return write.then(
      () => {
        log(`${verb} the ${scope} lease`);
        return true;
      },
      () => false,
    );
  }

  function sendLeaseRelease(token) {
    const writer = state.sessionWriter;
    if (!writer || !transportSessions.isActive(token)) return;
    if (authorityFor(gimbalScope).granted) {
      const action = state.controlShell?.planAuthority("gimbal", false);
      if (action) void executeLeaseAction(token, action, gimbalScope, writer);
    }
    if (releaseTracker.isPending()) return;
    const scope = state.motionScope;
    const action = state.controlShell?.planAuthority("motion", false);
    if (action !== "release") return;
    const settled = releaseTracker.begin();
    settled.then((outcome) => {
      transportSessions.runIfActive(token, () => log(`lease release ${outcome}`));
    });
    void executeLeaseAction(token, action, scope, writer).then((written) => {
      if (!written) releaseTracker.abandon();
    });
    log("sent LeaseRelease for " + scope);
  }

  function suspendControlForInputLoss(token) {
    state.stopControlRun?.stop();
    const releaseNeeded =
      authorityFor(state.motionScope).granted || state.resumePendingToken === token;
    state.resumeGimbalLease ||= authorityFor(gimbalScope).granted;
    if (releaseNeeded) sendLeaseRelease(token);
  }

  function showResumeAffordance() {
    const token = transportSessions.currentToken();
    const available =
      token !== null &&
      state.connected &&
      controlGate.isLatched() &&
      motionCapabilityFor(state.motionScope);
    els.resumeBtn.hidden = !available;
    if (available) surface.controlSuspended();
  }

  async function requestResumeLeases(token) {
    const writer = state.sessionWriter;
    if (!writer || !transportSessions.isActive(token)) {
      throw new Error("the live session stream is unavailable");
    }
    const motion = state.controlShell?.planAuthority("motion", true);
    if (!motion) throw new Error(resumeRefusalReason(authorityFor(state.motionScope)));
    await executeLeaseAction(token, motion, state.motionScope, writer);
    if (state.resumeGimbalLease) {
      const gimbal = state.controlShell?.planAuthority("gimbal", true);
      if (gimbal) await executeLeaseAction(token, gimbal, gimbalScope, writer);
    }
  }

  async function resumeControlInPlace() {
    const token = transportSessions.currentToken();
    if (!token || !state.connected) {
      els.resumeBtn.hidden = true;
      requestReconnect();
      return;
    }
    els.resumeBtn.disabled = true;
    try {
      const result = await resumeSessionControl({
        gate: controlGate,
        releases: releaseTracker,
        controlSettled: () => state.controlCompletion ?? Promise.resolve(),
        isSessionLive: () => transportSessions.isActive(token) && state.connected,
        announceActivation: () => maybeAnnounceProfileActivation(),
        requestLeases: async () => {
          state.resumePendingToken = token;
          await requestResumeLeases(token);
        },
        surrender: () => sendLeaseRelease(token),
      });
      if (!result.requested) {
        showResumeAffordance();
        return;
      }
      surface.resumeResult(result.interrupted);
    } catch (error) {
      state.resumePendingToken = null;
      controlGate.latchInputLoss();
      log(`same-session resume failed: ${error}`);
      showResumeAffordance();
    } finally {
      els.resumeBtn.disabled = false;
    }
  }

  function applyLeaseResponse(scope, message) {
    const slot = authoritySlot(scope);
    const disposition = applyAuthorityTransition(
      state.controlShell,
      log,
      slot,
      message.granted ? "grant" : "denial",
      { generation: message.generation, reason: message.reason },
    );
    if (disposition === "applied" && message.granted) {
      if (slot === "motion") state.sequence = 0;
      if (slot === "gimbal") state.gimbalSequence = 0;
    }
    return { disposition, ...authorityFor(scope) };
  }

  function completePendingResume(token, granted) {
    const decision = resumeGrantDecision({
      pending: state.resumePendingToken === token,
      granted,
      sessionLive: transportSessions.isActive(token) && state.connected,
      mayPublish: !controlGate.isLatched(),
    });
    if (decision === "unrelated") return;
    if (decision === "surrender") {
      suspendControlForInputLoss(token);
      state.resumePendingToken = null;
      showResumeAffordance();
      return;
    }
    state.resumePendingToken = null;
    if (decision === "denied") {
      if (authorityFor(gimbalScope).granted) {
        const action = state.controlShell?.planAuthority("gimbal", false);
        if (action) void executeLeaseAction(token, action, gimbalScope);
      }
      surface.resumeDenied();
      return;
    }
    state.resumeGimbalLease = false;
    if (startControlLoop(state.transport, token)) {
      els.resumeBtn.hidden = true;
      surface.controlResumed();
      log("same-session control resumed on fresh authority");
    } else {
      controlGate.latchInputLoss();
      suspendControlForInputLoss(token);
      showResumeAffordance();
    }
  }

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
        if (decoded.kind === "FrameRejected") {
          handleFrameRejected(decoded.message);
        }
        if (decoded.kind === "LeaseReleased") {
          const message = decoded.message;
          if (message.vehicleId === vehicleId && message.scope === state.motionScope) {
            releaseTracker.acknowledge();
            applyAuthorityTransition(state.controlShell, log, "motion", "release", {
              generation: message.generation,
            });
            if (state.pendingMotionScope) {
              state.motionScope = state.pendingMotionScope;
              state.pendingMotionScope = null;
              if (state.motionScope === directScope) {
                const heading = currentTelemetryHeading();
                state.fpvHeading = heading ?? 0;
                state.lastDirectFrameMs = 0;
              }
              log(`motion scope is now ${state.motionScope}`);
            }
          } else if (message.vehicleId === vehicleId && message.scope === gimbalScope) {
            applyAuthorityTransition(state.controlShell, log, "gimbal", "release", {
              generation: message.generation,
            });
          } else if (message.vehicleId === vehicleId && message.scope === lifecycleScope) {
            applyAuthorityTransition(state.controlShell, log, "lifecycle", "release", {
              generation: message.generation,
            });
          } else {
            log(`ignoring LeaseReleased for vehicle=${message.vehicleId} scope=${message.scope}`);
          }
        }
        if (
          decoded.kind === "LeaseResponse" &&
          decoded.message.scope === gimbalScope &&
          decoded.message.vehicleId === vehicleId
        ) {
          const message = decoded.message;
          const after = applyLeaseResponse(gimbalScope, message);
          if (
            after.granted &&
            (controlGate.isLatched() ||
              (state.resumeGimbalLease && authorityFor(state.motionScope).denied))
          ) {
            const action = state.controlShell?.planAuthority("gimbal", false);
            if (action) void executeLeaseAction(token, action, gimbalScope);
            log("input loss raced the gimbal grant; surrendered immediately");
          } else if (after.denied) {
            surface.gimbalDenied();
          }
        }
        if (
          decoded.kind === "LeaseResponse" &&
          decoded.message.scope === lifecycleScope &&
          decoded.message.vehicleId === vehicleId
        ) {
          const message = decoded.message;
          const after = applyLeaseResponse(lifecycleScope, message);
          if (after.granted && state.lifecycle.pendingPress) {
            state.lifecycle.pendingPress = false;
            if (requestAction(lifecycleScope, CONTROL_ACTION.simReset)) {
              log("simulation reset requested");
            }
          } else if (after.denied) {
            state.lifecycle.pendingPress = false;
            surface.lifecycleDenied();
          } else if (after.disposition === "stale") {
            const action = state.controlShell?.planAuthority("lifecycle", true);
            if (action) void executeLeaseAction(token, action, lifecycleScope);
          }
        }
        if (
          decoded.kind === "LeaseResponse" &&
          decoded.message.scope === state.motionScope &&
          decoded.message.vehicleId === vehicleId
        ) {
          const message = decoded.message;
          const after = applyLeaseResponse(state.motionScope, message);
          if (after.denied) {
            surface.motionDenied();
          } else if (after.disposition !== "stale") {
            state.fpvActive = state.motionScope === directScope;
            surface.motionScopeGranted(state.fpvActive);
          }
          if (after.disposition !== "stale") {
            completePendingResume(token, after.granted);
          }
        }
      }
    }
  }

  function dispatchAuthorityEnvelope(decoded, token) {
    if (!transportSessions.isActive(token)) return;
    if (decoded.kind === "AuthorityEvent") {
      surface.authorityNotice(decoded.message.arm);
      const message = decoded.message;
      const slot = message.scope === gimbalScope ? "gimbal"
        : message.scope === lifecycleScope ? "lifecycle"
          : motionGroup(message.scope) === motionGroup(state.motionScope) ? "motion" : null;
      if (
        slot &&
        message.vehicleId === vehicleId &&
        message.principalId === state.principalId &&
        message.kind
      ) {
        applyAuthorityTransition(state.controlShell, log, slot, message.kind, {
          generation: message.generation,
        });
      } else {
        // Not one of OUR table transitions (an override, a warning, a
        // transfer, or another principal's lease traffic): still a
        // correctness signal — log the raw arm rather than reducing it to a
        // transient overlay.
        log(`authority event: ${message.arm}`);
      }
    } else if (decoded.kind === "LinkLossCleared") {
      const message = decoded.message;
      if (message.vehicleId === vehicleId && message.scope === motionGroup(state.motionScope)) {
        applyAuthorityTransition(state.controlShell, log, "motion", "recovery", {
          generation: message.generation,
        });
      }
    } else if (decoded.kind === "VideoDeliveryState") {
      surface.videoDeliveryState(decoded.message);
    }
  }

  function handleFrameRejected(rejection) {
    const key = `${rejection.reason}:${rejection.scope}:${rejection.currentGeneration}`;
    const firstNotice = key !== state.lastFrameRejectionLogged;
    if (firstNotice) {
      state.lastFrameRejectionLogged = key;
      log(
        `control frame rejected (reason ${rejection.reason}) scope=${rejection.scope} ` +
          `seq=${rejection.sequence} hostGen=${rejection.currentGeneration}`,
      );
    }
    if (
      rejection.reason === frameRejectionUplinkIdle &&
      motionGroup(rejection.scope) === motionGroup(state.motionScope)
    ) {
      applyAuthorityTransition(state.controlShell, log, "motion", "uplinkIdle", {
        generation: rejection.currentGeneration,
      });
      if (firstNotice) surface.uplinkIdle();
    }
    if ((rejection.reason === 1 || rejection.reason === 2) && rejection.scope === gimbalScope) {
      applyAuthorityTransition(state.controlShell, log, "gimbal", "revocation", {
        generation: rejection.currentGeneration,
      });
    }
    if (
      (rejection.reason === 1 || rejection.reason === 2) &&
      motionGroup(rejection.scope) === motionGroup(state.motionScope)
    ) {
      applyAuthorityTransition(state.controlShell, log, "motion", "revocation", {
        generation: rejection.currentGeneration,
      });
      const action = state.controlShell?.planAuthority("motion", true);
      if (action) {
        void executeLeaseAction(transportSessions.currentToken(), action, state.motionScope);
      }
    }
  }

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

  function gamepadDisconnected(event) {
    if (!state.controlShell) return;
    if (state.selectedPadId !== null && event.gamepad?.id === state.selectedPadId) {
      state.selectedPadId = null;
      state.controlShell.deselectDevice();
      log(`gamepad disconnected: ${event.gamepad.id}; control returns to the keyboard`);
    }
  }

  function activeGamepad() {
    const pads = (navigator.getGamepads && navigator.getGamepads()) || [];
    for (const pad of pads) if (pad && pad.connected) return pad;
    return null;
  }

  function startControlLoop(transport, token) {
    state.controlShell?.beginControlRun();
    const runStop = createDatagramRunStop();
    const started = startDatagramControl({
      datagrams: transport.datagrams,
      lifecycle: transportSessions,
      token,
      run: (writer) => runControlLoop(writer, token, runStop),
      onError: (error) => log(`control loop stopped: ${error}`),
    });
    if (!started.ok) {
      const detail = started.reason === "send-stream-unavailable"
        ? "no datagram send stream"
        : "datagram writer acquisition failed";
      surface.controlUnavailable(detail);
      log(`control unavailable: ${detail}`);
    } else {
      const completion = started.completion;
      state.controlCompletion = completion;
      state.stopControlRun = runStop;
      completion.finally(() => {
        if (state.controlCompletion === completion) {
          state.controlCompletion = null;
          state.stopControlRun = null;
          if (transportSessions.isActive(token) && state.connected) startSuspendedPressWatch(token);
        }
      }).catch(() => {});
    }
    return started.ok;
  }

  function evaluateInputTick(mode) {
    const pad = activeGamepad();
    if (pad && pad.id !== state.selectedPadId) {
      state.selectedPadId = pad.id;
      const outcome = state.controlShell.selectDevice(pad.id);
      if (outcome === null) log(`gamepad REFUSED (ambiguous device-profile registry): ${pad.id}`);
      else log(`gamepad selected (${outcome}): ${pad.id}`);
    } else if (!pad && state.selectedPadId !== null) {
      state.selectedPadId = null;
      state.controlShell.deselectDevice();
      log("gamepad gone; control returns to the keyboard");
    }
    const sessionState = {
      mode,
      connected: state.connected, inputLost: controlGate.isLatched(),
      nowMs: performance.now(),
    };
    const plan = pad
      ? state.controlShell.tickFromPad(pad, sessionState)
      : state.controlShell.tickFromKeys(sessionState);
    return { pad, plan };
  }

  function startSuspendedPressWatch(token) {
    if (state.pressWatchToken === token) return;
    state.pressWatchToken = token;
    void (async () => {
      const intervalMs = 1000 / controlHz;
      while (
        transportSessions.isActive(token) &&
        state.connected &&
        state.controlCompletion === null
      ) {
        if (state.controlShell) {
          const mode = els.flightMode ? els.flightMode.value : "rover";
          const { plan } = evaluateInputTick(mode);
          reportSuppressedPresses(plan);
          reportExpiredActions();
        }
        await new Promise((resolve) => setTimeout(resolve, intervalMs));
      }
      if (state.pressWatchToken === token) state.pressWatchToken = null;
    })();
  }

  async function runControlLoop(writer, token, runStop) {
    const intervalMs = 1000 / controlHz;
    while (transportSessions.isActive(token) && state.connected) {
      const ready = await runStop.waitFor(writer.ready);
      if (!ready) return;
      if (!transportSessions.isActive(token) || !state.connected) return;
      const mode = els.flightMode ? els.flightMode.value : "rover";
      if (controlGate.isLatched()) {
        suspendControlForInputLoss(token);
        surface.controlReleased();
        log("input lost — control authority released (host acknowledges or watchdog covers)");
        showResumeAffordance();
        return;
      }
      if (!state.controlShell) {
        await new Promise((resolve) => setTimeout(resolve, intervalMs));
        continue;
      }
      const { pad, plan } = evaluateInputTick(mode);
      updateControlReadout(pad, mode, plan);
      reportSuppressedPresses(plan);
      reportExpiredActions();
      if (state.pendingReset) {
        state.pendingReset = false;
        requestSimReset();
      }
      maybeAnnounceProfileActivation();
      if (!controlGate.mayPublish()) continue;
      if (plan.motion) sendMotionFrame(writer, token, mode, plan);
      if (plan.gimbal) sendGimbalFrame(writer, token, plan.gimbal);
      if (plan.lease) executeLeaseAction(token, plan.lease, gimbalScope);
      if (plan.motionLease) executeLeaseAction(token, plan.motionLease, state.motionScope);
      await new Promise((resolve) => setTimeout(resolve, intervalMs));
    }
  }

  function velocityCapabilityFor(scope) {
    return capabilityFor(state.advertisedScopes, vehicleId, scope, INTENT_FAMILY_VELOCITY);
  }

  function motionCapabilityFor(scope) {
    const family = scope === directScope
      ? INTENT_FAMILY_ATTITUDE_THRUST
      : INTENT_FAMILY_VELOCITY;
    return capabilityFor(state.advertisedScopes, vehicleId, scope, family);
  }

  function generationForScope(scope) {
    return authorityFor(scope).generation;
  }

  function requestSimReset() {
    if (!actionAdvertised(state.advertisedScopes, vehicleId, lifecycleScope, CONTROL_ACTION.simReset)) {
      log("sim reset not advertised (not a simulator host); not sent");
      return;
    }
    if (authorityFor(lifecycleScope).granted) {
      if (requestAction(lifecycleScope, CONTROL_ACTION.simReset)) log("simulation reset requested");
      return;
    }
    state.lifecycle.pendingPress = true;
    const action = state.controlShell?.planAuthority("lifecycle", true);
    if (action) void executeLeaseAction(transportSessions.currentToken(), action, lifecycleScope);
  }

  function requestAction(scope, action, modeTarget, cancels = []) {
    if (!actionAdvertised(state.advertisedScopes, vehicleId, scope, action, modeTarget)) {
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
      vehicleId,
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

  function handleActionResult(message) {
    const verdict = message.accepted ? "accepted" : `REJECTED (${message.detail})`;
    const id = message.actionId ? ` id=${message.actionId}` : "";
    log(`action result [${message.scope} seq=${message.sequence}] action=${message.action}${id} ${verdict}`);
    if (!message.actionId) return;
    const entry = resolveAction(state.actionTracker, message.actionId);
    if (!entry) return;
    if (motionGroup(entry.scope) === motionGroup(state.motionScope)) {
      const before = authorityFor(state.motionScope);
      applyAuthorityTransition(state.controlShell, log, "motion", "actionResult", {
        detail: entry.action,
        accepted: message.accepted,
      });
      if (before.needsArm && !authorityFor(state.motionScope).needsArm) {
        state.lastFrameRejectionLogged = null;
        surface.armAccepted();
      }
    }
    if (entry.scope === lifecycleScope) {
      const action = state.controlShell?.planAuthority("lifecycle", false);
      if (action) void executeLeaseAction(transportSessions.currentToken(), action, lifecycleScope);
    }
    if (entry.action === CONTROL_ACTION.modeRequest && message.accepted) {
      state.fpvActive = entry.modeTarget === MODE_TARGET.fpvDirect;
      surface.modeEngaged(state.fpvActive);
      log(`mode ack: ${state.fpvActive ? "fpv-direct" : "camera-velocity"} engaged`);
    }
  }

  function reportExpiredActions() {
    for (const gone of expirePending(state.actionTracker, performance.now())) {
      log(
        `action ${gone.action} (id ${gone.actionId}) got no result within ` +
          `${ACTION_TIMEOUT_MS} ms; the session stream may be dead`,
      );
    }
  }

  function sendMotionFrame(writer, token, mode, plan) {
    const direct = state.motionScope === directScope;
    const capability = motionCapabilityFor(state.motionScope);
    if (!capability) return;
    const motion = plan.motion;
    if (
      plan.arm &&
      requestAction(state.motionScope, CONTROL_ACTION.arm, undefined, [CONTROL_ACTION.disarm])
    ) {
      surface.commandSent(true);
      log("arm command sent");
    }
    if (
      plan.disarm &&
      requestAction(state.motionScope, CONTROL_ACTION.disarm, undefined, [CONTROL_ACTION.arm])
    ) {
      surface.commandSent(false);
      log("disarm command sent");
    }
    if (state.pendingFpvToggle) {
      state.pendingFpvToggle = false;
      requestFlightModeSwitch();
    }
    let velocity;
    let attitudeThrust;
    if (direct) {
      const nowMs = performance.now();
      const dt = state.lastDirectFrameMs
        ? Math.min((nowMs - state.lastDirectFrameMs) / 1000, 0.1)
        : 0;
      state.lastDirectFrameMs = nowMs;
      state.fpvHeading = integrateHeading(state.fpvHeading, motion.yaw, capability, dt);
      attitudeThrust = buildAttitudeThrustIntent(motion, state.fpvHeading, capability);
    } else {
      velocity = buildVelocityIntent(motion, mode, capability);
    }
    state.sequence = (state.sequence + 1) >>> 0;
    const envelope = encodeControlFrameEnvelope({
      sessionId: state.sessionId,
      vehicleId,
      scope: state.motionScope,
      generation: authorityFor(state.motionScope).generation,
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

  function requestFlightModeSwitch() {
    const target = state.motionScope === directScope ? motionScope : directScope;
    const family = target === directScope
      ? INTENT_FAMILY_ATTITUDE_THRUST
      : INTENT_FAMILY_VELOCITY;
    if (capabilityFor(state.advertisedScopes, vehicleId, target, family)) {
      if (state.pendingMotionScope) return;
      if (target === directScope && currentTelemetryHeading() === null) {
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
    const fpvBase = pendingTarget !== undefined
      ? pendingTarget === MODE_TARGET.fpvDirect
      : state.fpvActive;
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

  function sendGimbalFrame(writer, token, gimbal) {
    const capability = capabilityFor(
      state.advertisedScopes,
      vehicleId,
      gimbalScope,
      INTENT_FAMILY_GIMBAL_RATE,
    );
    if (!capability) return;
    if (gimbal.recenter && requestAction(gimbalScope, CONTROL_ACTION.gimbalRecenter)) {
      log("gimbal recenter requested (R3)");
    }
    state.gimbalSequence = (state.gimbalSequence + 1) >>> 0;
    const envelope = encodeControlFrameEnvelope({
      sessionId: state.sessionId,
      vehicleId,
      scope: gimbalScope,
      generation: authorityFor(gimbalScope).generation,
      sequence: state.gimbalSequence,
      sampledAtNanos: nowNanos(),
      profileRevision: state.controlShell.profileRevision(),
      activationRevision: state.controlShell.activationRevision(),
      gimbalRate: buildGimbalRateIntent(gimbal, capability),
    });
    writer.write(envelope).catch((error) => {
      transportSessions.runIfActive(token, () => log(`gimbal datagram send failed: ${error}`));
    });
  }

  async function startControl() {
    try {
      const wasmSource = await fetch("./control-runtime_bg.wasm", { cache: "no-cache" });
      state.controlShell = await loadControlShell(wasmSource);
      if (transportSessions.active !== null) state.controlShell.beginSession();
      log(`control runtime ready (profile revision ${state.controlShell.activationRevision()})`);
    } catch (error) {
      log(`control runtime unavailable: ${error} (run scripts/build-web-instruments.sh)`);
    }
  }

  return {
    applyLeaseResponse,
    authorityFor,
    dispatchAuthorityEnvelope,
    executeLeaseAction,
    forwardKey,
    gamepadDisconnected,
    handleActionResult,
    handleFrameRejected,
    motionGroup,
    resumeControlInPlace,
    runSessionStreamReader,
    sendLeaseRelease,
    showResumeAffordance,
    startControl,
    startControlLoop,
    startSuspendedPressWatch,
    suspendControlForInputLoss,
    velocityCapabilityFor,
  };
}
