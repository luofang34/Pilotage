// Cooperative handover, the holder's half (CLIENT-09): another operator
// asked for a scope this client holds; the operator decides, and nothing
// changes hands until they do. Kept out of the control loop so the loop
// stays about frames, and out of wire.js so its cap stays honest.

import { encodeScopeTransferOfferEnvelope } from "./wire.js";

/**
 * Handles one decoded `ScopeTransferRequested` addressed at a scope this
 * client holds. `confirmDecision` is injectable for tests; the browser
 * default is the one modal decision surface the viewer has. Returns true
 * when the event was a transfer request (handled either way).
 */
export function handleTransferRequest({
  message,
  vehicleId,
  granted,
  writer,
  frame,
  log,
  confirmDecision = (prompt) => globalThis.confirm(prompt),
}) {
  if (message.kind !== "transferRequest") return false;
  if (message.vehicleId !== vehicleId || !granted) return true;
  const handOver = confirmDecision(
    `Operator ${message.principalId} asks for ${message.scope} — hand over control?`,
  );
  if (handOver && writer) {
    const envelope = encodeScopeTransferOfferEnvelope({
      vehicleId: message.vehicleId,
      scope: message.scope,
      toPrincipal: message.principalId,
    });
    writer.write(frame(envelope)).then(
      () => log(`offered ${message.scope} to operator ${message.principalId}`),
      (error) => log(`transfer offer failed: ${error}`),
    );
  } else {
    log(`kept ${message.scope}; the ask from operator ${message.principalId} expires`);
  }
  return true;
}
