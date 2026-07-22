import { encodeLeaseReleaseEnvelope, encodeLeaseRequestEnvelope } from "./wire.js";

/** Encodes and writes one runtime-planned lease action. */
export function writeLeaseAction({ writer, action, vehicleId, scope, frame }) {
  let envelope;
  if (action === "request") {
    envelope = encodeLeaseRequestEnvelope({ vehicleId, scope });
  } else if (action === "release") {
    envelope = encodeLeaseReleaseEnvelope({ vehicleId, scope });
  } else {
    return null;
  }
  return writer.write(frame(envelope));
}
