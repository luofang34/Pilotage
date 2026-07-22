import {
  encodeClientHelloEnvelope,
  encodeProfileActivationEnvelope,
} from "./wire.js";
import { createActionTracker } from "./action-tracker.js";

/** Builds the reliable-session bootstrap and activation announcer. */
export function createSessionBootstrap({
  state,
  surface,
  transportSessions,
  motionScope,
  controlStarted,
  log,
  authorityFor,
  executeLeaseAction,
  velocityCapabilityFor,
  handleActionResult,
  applyLeaseResponse,
}) {
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

  async function sendLeaseRequest(writer, token) {
    await controlStarted();
    if (!transportSessions.isActive(token)) return false;
    const action = state.controlShell?.planAuthority("motion", true);
    if (action !== "request") return authorityFor(motionScope).granted;
    if (!(await executeLeaseAction(token, action, motionScope, writer))) return false;
    if (!transportSessions.isActive(token)) return false;
    log(`sent LeaseRequest for ${motionScope}`);
    return true;
  }

  function handleBootstrapMessage(decoded, token) {
    if (!transportSessions.isActive(token)) return;
    if (decoded.kind === "ServerWelcome") {
      state.sessionId = decoded.message.sessionId;
      state.principalId = decoded.message.principalId;
      state.advertisedScopes = decoded.message.advertisedScopes ?? [];
      state.actionTracker = createActionTracker();
      state.announcedActivationRevision = null;
      state.motionScope = motionScope;
      state.pendingMotionScope = null;
      state.fpvActive = false;
      state.lifecycle = { pendingPress: false };
      log(`ServerWelcome: session=${decoded.message.sessionId} principal=${decoded.message.principalId}`);
      for (const scope of state.advertisedScopes) {
        const families = scope.intents.map((intent) => intent.family).join(",");
        log(`capability: ${scope.scope} intents=[${families}] actions=${scope.actions.length}`);
      }
      if (!velocityCapabilityFor(motionScope)) {
        log("vehicle advertises no velocity intent for vehicle.motion; motion control disabled");
      }
    } else if (decoded.kind === "ControlActionResult") {
      handleActionResult(decoded.message);
    } else if (decoded.kind === "LeaseResponse") {
      if (decoded.message.scope && decoded.message.scope !== motionScope) {
        log(`ignoring bootstrap LeaseResponse for scope=${decoded.message.scope}`);
        return;
      }
      applyLeaseResponse(motionScope, decoded.message);
      if (!decoded.message.granted) {
        surface.leaseDenied(decoded.message.reason);
      }
    }
  }

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

  function maybeAnnounceProfileActivation() {
    if (!state.sessionWriter || !state.controlShell) return;
    if (state.controlShell.activationRevision() === state.announcedActivationRevision) return;
    announceProfileActivation();
  }

  return {
    announceProfileActivation,
    handleBootstrapMessage,
    lengthDelimit,
    maybeAnnounceProfileActivation,
    sendClientHello,
    sendLeaseRequest,
  };
}
