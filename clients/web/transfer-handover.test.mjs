// The holder's confirm must produce exactly the offer the host expects:
// envelope arm 22 carrying the asker's identity — no silent principal or
// scope drift between the dialog and the wire.
import assert from "node:assert/strict";
import test from "node:test";

import { handleTransferRequest } from "./transfer-handover.js";

function varint(bytes, at) {
  let value = 0n;
  let shift = 0n;
  let index = at;
  for (;;) {
    const byte = BigInt(bytes[index]);
    value |= (byte & 0x7fn) << shift;
    index += 1;
    if ((byte & 0x80n) === 0n) return [value, index];
    shift += 7n;
  }
}

test("a confirmed ask becomes a ScopeTransferOffer to the asker", async () => {
  const written = [];
  const writer = { write: (frame) => (written.push(frame), Promise.resolve()) };
  const handled = handleTransferRequest({
    message: {
      kind: "transferRequest",
      principalId: 7n,
      vehicleId: 1n,
      scope: "vehicle.motion",
    },
    vehicleId: 1n,
    granted: true,
    writer,
    frame: (bytes) => bytes,
    log: () => {},
    confirmDecision: () => true,
  });
  assert.equal(handled, true);
  assert.equal(written.length, 1);
  const bytes = written[0];
  // Envelope: schema_version (field 1 varint), then arm 22 (field 22, wire 2).
  let [tag, at] = varint(bytes, 0);
  assert.equal(tag, (1n << 3n) | 0n, "schema version leads");
  let version;
  [version, at] = varint(bytes, at);
  assert.equal(version, 1n);
  [tag, at] = varint(bytes, at);
  assert.equal(tag, (22n << 3n) | 2n, "the payload is arm 22, ScopeTransferOffer");
  let length;
  [length, at] = varint(bytes, at);
  const body = bytes.slice(at, at + Number(length));
  // to_principal (field 3) carries the asker's id 7.
  const text = Array.from(body, (b) => b.toString(16).padStart(2, "0")).join("");
  assert.ok(text.includes("1a020807"), `field 3 must carry principal 7: ${text}`);
});

test("a declined ask writes nothing and still counts as handled", () => {
  const written = [];
  const handled = handleTransferRequest({
    message: { kind: "transferRequest", principalId: 7n, vehicleId: 1n, scope: "s" },
    vehicleId: 1n,
    granted: true,
    writer: { write: (frame) => (written.push(frame), Promise.resolve()) },
    frame: (bytes) => bytes,
    log: () => {},
    confirmDecision: () => false,
  });
  assert.equal(handled, true);
  assert.equal(written.length, 0);
});
