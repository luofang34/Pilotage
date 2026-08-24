// Device and source-axis checks for browser characterization captures.
const DIGEST = /^[0-9a-f]{64}$/;
const WELL_FORMED_UTF16 = /^(?:[\u0000-\uD7FF\uE000-\uFFFF]|[\uD800-\uDBFF][\uDC00-\uDFFF])*$/;
const NEUTRAL_POSITIONS = new Set(["centered", "minimum", "maximum"]);
/** Creates one immutable binding to a Gamepad connection and axis contract. */
export function bindGamepadContract({
  device,
  gamepad,
  connectionEpoch,
  sourceContractDigest,
  sourceAxes,
}) {
  if (!isWellFormedText(connectionEpoch) || connectionEpoch.length === 0 ||
      new TextEncoder().encode(connectionEpoch).byteLength > 256) {
    throw new TypeError("connectionEpoch must identify one connection");
  }
  if (!DIGEST.test(sourceContractDigest)) {
    throw new TypeError("sourceContractDigest must be lowercase SHA-256");
  }
  const metadata = copyGamepadMetadata(gamepad);
  if (!metadata.connected) throw new Error("the Gamepad is disconnected");
  const copiedDevice = copyDevice(device, metadata.id);
  const copiedAxes = copySourceAxes(sourceAxes, gamepad?.axes?.length);
  return Object.freeze({
    device: Object.freeze(copiedDevice),
    gamepad,
    metadata: Object.freeze(metadata),
    connectionEpoch,
    sourceContractDigest,
    sourceAxes: Object.freeze(copiedAxes.map(Object.freeze)),
  });
}

/** Rejects a sample from a changed, replaced, or disconnected Gamepad. */
export function assertGamepadBinding(binding, gamepad, connectionEpoch) {
  if (gamepad !== binding.gamepad || connectionEpoch !== binding.connectionEpoch) {
    throw new Error("the Gamepad connection instance changed");
  }
  const current = copyGamepadMetadata(gamepad);
  if (!current.connected) throw new Error("the Gamepad is disconnected");
  for (const key of ["id", "index", "mapping"]) {
    if (current[key] !== binding.metadata[key]) {
      throw new Error(`the Gamepad ${key} changed`);
    }
  }
  if (gamepad.axes.length !== binding.sourceAxes.length) {
    throw new Error("the Gamepad axis count changed");
  }
}

function copyGamepadMetadata(gamepad) {
  if (typeof gamepad !== "object" || gamepad === null) throw new TypeError("gamepad is required");
  if (typeof gamepad.id !== "string" || gamepad.id.length === 0 || gamepad.id.length > 512) {
    throw new TypeError("invalid Gamepad id");
  }
  if (!Number.isSafeInteger(gamepad.index) || gamepad.index < 0) throw new TypeError("invalid Gamepad index");
  if (typeof gamepad.mapping !== "string") throw new TypeError("invalid Gamepad mapping");
  return { id: gamepad.id, index: gamepad.index, mapping: gamepad.mapping, connected: gamepad.connected === true };
}

function copyDevice(device, gamepadId) {
  const vendor = Number(device?.vendor_id);
  const product = Number(device?.product_id);
  if (!Number.isInteger(vendor) || vendor < 0 || vendor > 0xffff) throw new TypeError("invalid device vendor_id");
  if (!Number.isInteger(product) || product < 0 || product > 0xffff) throw new TypeError("invalid device product_id");
  if (vendor === 0 && product === 0) throw new Error("wildcard device identity is not evidence");
  const parsed = parseUsbIdentity(gamepadId);
  if (parsed === null || parsed.vendor !== vendor || parsed.product !== product) {
    throw new Error("device identity does not match Gamepad.id");
  }
  const name = typeof device.product === "string" ? device.product : null;
  if (name !== null && (!isWellFormedText(name) || name.length === 0 || new TextEncoder().encode(name).byteLength > 256)) throw new TypeError("invalid device product name");
  return {
    vendor_id: vendor,
    product_id: product,
    product: name,
  };
}

function copySourceAxes(sourceAxes, gamepadAxisCount) {
  if (!Array.isArray(sourceAxes) || sourceAxes.length === 0 || sourceAxes.length > 64) {
    throw new TypeError("invalid source axis contract");
  }
  if (sourceAxes.length !== gamepadAxisCount) throw new Error("source contract axis count differs from Gamepad");
  return sourceAxes.map((axis, index) => {
    if (axis?.source_index !== index || axis.minimum !== -1 || axis.maximum !== 1 ||
        !NEUTRAL_POSITIONS.has(axis.neutral_position)) {
      throw new TypeError("invalid browser source axis contract");
    }
    return { source_index: index, minimum: -1, maximum: 1, neutral_position: axis.neutral_position };
  });
}

function parseUsbIdentity(id) {
  const chromium = /Vendor:\s*([0-9a-f]{4}).*Product:\s*([0-9a-f]{4})/i.exec(id);
  const firefox = /^([0-9a-f]{4})-([0-9a-f]{4})-/i.exec(id);
  const match = chromium ?? firefox;
  return match === null
    ? null
    : { vendor: Number.parseInt(match[1], 16), product: Number.parseInt(match[2], 16) };
}
export function isWellFormedText(value) {
  return typeof value === "string" && WELL_FORMED_UTF16.test(value);
}
