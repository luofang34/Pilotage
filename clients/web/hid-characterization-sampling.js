// Browser Gamepad sampling bridge for the shared HID characterization schema.
// The bridge records only a new Gamepad timestamp. Animation frames with the
// same report timestamp do not change report-period evidence.

const SCHEMA_VERSION = 1;

function defaultDeadzoneEvidence() {
  return Object.freeze({
    status: "unknown",
    method: "unmeasured",
    sample_count: 0,
  });
}

function validateDeadzoneEvidence(evidence) {
  const { status, method, sample_count: count } = evidence;
  const valid =
    (status === "unknown" && method === "unmeasured" && count === 0) ||
    ((status === "observed" || status === "not_observed") &&
      method === "paired_native_and_platform" &&
      Number.isSafeInteger(count) && count > 0);
  if (!valid) throw new TypeError("invalid platform dead-zone evidence");
}

function copyDevice(device) {
  const vendor = Number(device?.vendor_id);
  const product = Number(device?.product_id);
  if (!Number.isInteger(vendor) || vendor < 0 || vendor > 0xffff) {
    throw new TypeError("invalid device vendor_id");
  }
  if (!Number.isInteger(product) || product < 0 || product > 0xffff) {
    throw new TypeError("invalid device product_id");
  }
  return {
    vendor_id: vendor,
    product_id: product,
    product: typeof device.product === "string" ? device.product : null,
  };
}

/** Records browser Gamepad reports into the portable characterization schema. */
export class BrowserHIDCharacterizationSampler {
  constructor(device, deadzoneEvidence = defaultDeadzoneEvidence()) {
    validateDeadzoneEvidence(deadzoneEvidence);
    this.device = copyDevice(device);
    this.deadzoneEvidence = { ...deadzoneEvidence };
    this.samples = [];
    this.segments = [];
    this.openSegment = null;
    this.lastSourceTimestampMs = null;
    this.lastObservedAtUs = null;
  }

  /** Starts the one required idle segment. */
  beginIdle() {
    this.#begin({ kind: "idle" });
  }

  /** Starts one named movement. The positive direction must move first. */
  beginMovement(logical) {
    if (typeof logical !== "string" || logical.length === 0) {
      throw new TypeError("movement logical name is required");
    }
    this.#begin({ kind: "movement", logical, positive_first: true });
  }

  /** Records one new Gamepad report. Returns false for a repeated timestamp. */
  record(gamepad, observedAtUs) {
    if (this.openSegment === null) throw new Error("start a segment before sampling");
    if (!Number.isSafeInteger(observedAtUs) || observedAtUs < 0) {
      throw new TypeError("observedAtUs must be a non-negative integer");
    }
    if (this.lastObservedAtUs !== null && observedAtUs <= this.lastObservedAtUs) {
      throw new RangeError("arrival timestamps must increase");
    }
    const sourceMs = Number(gamepad?.timestamp);
    if (!Number.isFinite(sourceMs) || sourceMs < 0 || !Number.isSafeInteger(Math.round(sourceMs * 1000))) {
      throw new TypeError("Gamepad.timestamp is required for report-timing evidence");
    }
    if (sourceMs === this.lastSourceTimestampMs) return false;
    if (this.lastSourceTimestampMs !== null && sourceMs < this.lastSourceTimestampMs) {
      throw new RangeError("Gamepad timestamps must not decrease");
    }
    const axes = Array.from(gamepad?.axes ?? [], Number);
    if (axes.length === 0 || axes.some((axis) => !Number.isFinite(axis))) {
      throw new TypeError("Gamepad axes must be finite and non-empty");
    }
    this.samples.push({
      sequence: this.samples.length,
      observed_at_us: observedAtUs,
      source_at_us: Math.round(sourceMs * 1000),
      axes,
      report_hex: null,
    });
    this.lastObservedAtUs = observedAtUs;
    this.lastSourceTimestampMs = sourceMs;
    return true;
  }

  /** Closes the current segment after at least one new report. */
  endSegment() {
    if (this.openSegment === null) throw new Error("no segment is open");
    if (this.samples.length === this.openSegment.sampleStart) {
      throw new Error("a segment needs at least one new report");
    }
    this.segments.push({
      action: this.openSegment.action,
      start_sequence: this.openSegment.sampleStart,
      end_sequence: this.samples.length - 1,
    });
    this.openSegment = null;
  }

  /** Returns a JSON-ready portable capture. */
  finish() {
    if (this.openSegment !== null) throw new Error("close the current segment first");
    if (this.samples.length === 0 || this.segments.length === 0) {
      throw new Error("the capture is empty");
    }
    return {
      schema_version: SCHEMA_VERSION,
      device: { ...this.device },
      source: "browser_gamepad",
      timestamp_source: "source",
      deadzone_evidence: { ...this.deadzoneEvidence },
      samples: this.samples.map((sample) => ({ ...sample, axes: [...sample.axes] })),
      segments: this.segments.map((segment) => ({
        ...segment,
        action: { ...segment.action },
      })),
    };
  }

  #begin(action) {
    if (this.openSegment !== null) throw new Error("close the current segment first");
    this.openSegment = { action, sampleStart: this.samples.length };
  }
}
