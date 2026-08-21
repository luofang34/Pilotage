// Browser Gamepad sampling for the shared HID characterization schema.
import { assertGamepadBinding, bindGamepadContract, isWellFormedText } from "./hid-characterization-contract.js";
const SCHEMA_VERSION = 1;
const MAX_SAMPLES = 1_000_000;
const MAX_SEGMENTS = 65;
const MAX_LOGICAL_NAME_BYTES = 64;
const MAX_CAPTURE_BYTES = 64 * 1024 * 1024;
/** Records changed browser Gamepad states without claiming raw HID loss. */
export class BrowserHIDCharacterizationSampler {
  constructor(options) {
    this.binding = bindGamepadContract(options);
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
    if (!isWellFormedText(logical) || logical.length === 0 ||
        new TextEncoder().encode(logical).length > MAX_LOGICAL_NAME_BYTES) {
      throw new TypeError("movement logical name is invalid");
    }
    this.#begin({ kind: "movement", logical, positive_first: true });
  }

  /** Records one changed Gamepad state. Returns false for a repeated timestamp. */
  record(gamepad, connectionEpoch, observedAtUs) {
    if (this.openSegment === null) throw new Error("start a segment before sampling");
    assertGamepadBinding(this.binding, gamepad, connectionEpoch);
    if (!Number.isSafeInteger(observedAtUs) || observedAtUs < 0) {
      throw new TypeError("observedAtUs must be a non-negative integer");
    }
    if (this.lastObservedAtUs !== null && observedAtUs <= this.lastObservedAtUs) {
      throw new RangeError("arrival timestamps must increase");
    }
    const sourceMs = Number(gamepad.timestamp);
    const sourceUs = Math.round(sourceMs * 1000);
    if (!Number.isFinite(sourceMs) || sourceMs < 0 || !Number.isSafeInteger(sourceUs)) {
      throw new TypeError("Gamepad.timestamp is required for state-update timing");
    }
    if (sourceMs === this.lastSourceTimestampMs) return false;
    if (this.lastSourceTimestampMs !== null && sourceMs < this.lastSourceTimestampMs) {
      throw new RangeError("Gamepad timestamps must not decrease");
    }
    if (this.samples.length >= MAX_SAMPLES) throw new RangeError("capture sample limit reached");
    const axes = Array.from(gamepad.axes, Number);
    if (axes.some((axis) => !Number.isFinite(axis) || axis < -1 || axis > 1)) {
      throw new TypeError("Gamepad axes must be finite values in [-1, 1]");
    }
    this.samples.push({
      sequence: this.samples.length,
      observed_at_us: observedAtUs,
      source_at_us: sourceUs,
      axes,
      report_hex: null,
    });
    this.lastObservedAtUs = observedAtUs;
    this.lastSourceTimestampMs = sourceMs;
    return true;
  }

  /** Closes the current segment after at least one changed state. */
  endSegment() {
    if (this.openSegment === null) throw new Error("no segment is open");
    if (this.samples.length === this.openSegment.sampleStart) {
      throw new Error("a segment needs at least one changed state");
    }
    if (this.segments.length >= MAX_SEGMENTS) throw new RangeError("capture segment limit reached");
    this.segments.push({
      action: this.openSegment.action,
      start_sequence: this.openSegment.sampleStart,
      end_sequence: this.samples.length - 1,
    });
    this.openSegment = null;
  }

  /** Returns a JSON-ready portable capture. */
  finish(maximumCaptureBytes = MAX_CAPTURE_BYTES) {
    if (!Number.isSafeInteger(maximumCaptureBytes) || maximumCaptureBytes <= 0 || maximumCaptureBytes > MAX_CAPTURE_BYTES) throw new RangeError("invalid encoded capture byte limit");
    if (this.openSegment !== null) throw new Error("close the current segment first");
    if (this.samples.length === 0 || this.segments.length === 0) {
      throw new Error("the capture is empty");
    }
    const binding = this.binding;
    const capture = {
      schema_version: SCHEMA_VERSION,
      device: { ...binding.device },
      device_instance_id: binding.connectionEpoch,
      source: "browser_gamepad",
      timestamp_source: "source",
      timing_observation: "polled_state_updates",
      deadzone_evidence: { status: "unknown", method: "unmeasured", sample_count: 0 },
      source_contract_digest: binding.sourceContractDigest,
      source_axes: binding.sourceAxes.map((axis) => ({ ...axis })),
      samples: this.samples.map((sample) => ({ ...sample, axes: [...sample.axes] })),
      segments: this.segments.map((segment) => ({
        ...segment,
        action: { ...segment.action },
      })),
    };
    if (new TextEncoder().encode(`${JSON.stringify(capture)}\n`).byteLength > maximumCaptureBytes) throw new RangeError("encoded capture byte limit reached");
    return capture;
  }

  #begin(action) {
    if (this.openSegment !== null) throw new Error("close the current segment first");
    this.openSegment = { action, sampleStart: this.samples.length };
  }
}
