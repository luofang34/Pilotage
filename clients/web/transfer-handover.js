// Cooperative handover, the holder's half (CLIENT-09): another operator
// asked for a scope this client holds; the operator decides, and nothing
// changes hands until they do. Kept out of the control loop so the loop
// stays about frames, and out of wire.js so its cap stays honest.

import { encodeScopeTransferOfferEnvelope } from "./wire.js";

/**
 * Handles one decoded `ScopeTransferRequested` addressed at a scope this
 * client holds. `confirmDecision` is injectable for tests; the browser
 * default is a page-owned bar, NEVER `window.confirm` — a native dialog
 * blurs the window, the input-loss latch releases every held lease on
 * blur, and the offer then leaves a client that no longer holds anything.
 * Returns true when the event was a transfer request (handled either way).
 */
export function handleTransferRequest({
  message,
  vehicleId,
  granted,
  writer,
  frame,
  log,
  confirmDecision = confirmWithBar,
}) {
  if (message.kind !== "transferRequest") return false;
  if (message.vehicleId !== vehicleId || !granted) return true;
  const sendOffer = () => {
    if (!writer) return;
    const envelope = encodeScopeTransferOfferEnvelope({
      vehicleId: message.vehicleId,
      scope: message.scope,
      toPrincipal: message.principalId,
    });
    writer.write(frame(envelope)).then(
      () => log(`offered ${message.scope} to operator ${message.principalId}`),
      (error) => log(`transfer offer failed: ${error}`),
    );
  };
  const decision = confirmDecision(
    `Operator ${message.principalId} asks for ${message.scope} — hand over control?`,
  );
  Promise.resolve(decision).then((handOver) => {
    if (handOver) {
      sendOffer();
    } else {
      log(`kept ${message.scope}; the ask from operator ${message.principalId} expires`);
    }
  });
  return true;
}

/** One in-page decision bar; resolves without ever blurring the window. */
function confirmWithBar(prompt) {
  const existing = globalThis.document?.getElementById("transfer-handover-bar");
  existing?.remove();
  const doc = globalThis.document;
  if (!doc) return Promise.resolve(false);
  return new Promise((resolve) => {
    const bar = doc.createElement("div");
    bar.id = "transfer-handover-bar";
    bar.style.cssText =
      "position:fixed;top:0;left:0;right:0;z-index:1000;display:flex;gap:12px;" +
      "align-items:center;justify-content:center;padding:10px;background:#7a4a00;" +
      "color:#fff;font:14px system-ui";
    const label = doc.createElement("span");
    label.textContent = prompt;
    const finish = (answer) => {
      bar.remove();
      resolve(answer);
    };
    const yes = doc.createElement("button");
    yes.textContent = "Hand over";
    yes.addEventListener("click", () => finish(true));
    const no = doc.createElement("button");
    no.textContent = "Keep control";
    no.addEventListener("click", () => finish(false));
    bar.append(label, yes, no);
    doc.body.append(bar);
    // The host expires the offer window; the bar should not outlive it.
    setTimeout(() => {
      if (bar.isConnected) finish(false);
    }, 25_000);
  });
}
