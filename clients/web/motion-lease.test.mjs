// The single lease writer accepts runtime-planned actions for every scope and
// refuses unknown actions without touching the reliable stream.

import { writeLeaseAction } from "./lease-executor.js";

const writes = [];
const writer = {
  write(bytes) {
    writes.push(bytes);
    return Promise.resolve();
  },
};
const base = {
  writer,
  vehicleId: 1n,
  scope: "vehicle.motion",
  frame: (bytes) => bytes,
};

await writeLeaseAction({ ...base, action: "request" });
await writeLeaseAction({ ...base, action: "release" });
const ignored = writeLeaseAction({ ...base, action: "unknown" });

if (
  writes.length !== 2 ||
  writes[0].length === 0 ||
  writes[1].length === 0 ||
  ignored !== null
) {
  console.error("FAIL - lease executor did not preserve its one-writer contract");
  process.exit(1);
}

console.log("single lease executor contract passed");
