// H.264 (Annex-B) decode groundwork for the viewer, behind the same capture
// identity gate as MJPEG (ADR-0016 codec FourCC dispatch). The host does not
// emit H.264 yet; this is the decode side of that path so a host that adopts it
// renders without a viewer change, and a browser lacking WebCodecs (or the
// stream's profile) fails closed with a typed log rather than a blank canvas.
//
// The bitstream is decoded by the browser's WebCodecs VideoDecoder, not by hand
// in JS: this module only classifies Annex-B NAL units (to mark keyframes and
// derive the decoder config), feeds chunks, and blits the decoded frames — the
// transport/paint role the viewer keeps.

/** FourCC the host tags an H.264 Annex-B video body with (ADR-0016). */
export const FOURCC_H264 = "H264";

// H.264 NAL unit types read from the byte after each Annex-B start code
// (nal_unit_type = byte & 0x1F): 5 = IDR slice (keyframe), 7 = SPS.
const NAL_IDR = 5;
const NAL_SPS = 7;

/**
 * Iterates the `nal_unit_type` of every NAL unit in an Annex-B buffer,
 * invoking `onNal(type, bodyOffset)` for each. Recognizes both the 3-byte
 * (0x000001) and 4-byte (0x00000001) start codes. Malformed input simply
 * yields no NAL callbacks — the caller then treats the access unit as
 * undecodable (fail closed).
 */
export function forEachNalType(bytes, onNal) {
  let i = 0;
  const n = bytes.length;
  while (i + 3 <= n) {
    const isStart3 = bytes[i] === 0 && bytes[i + 1] === 0 && bytes[i + 2] === 1;
    const isStart4 =
      i + 4 <= n && bytes[i] === 0 && bytes[i + 1] === 0 && bytes[i + 2] === 0 && bytes[i + 3] === 1;
    if (isStart4) {
      const at = i + 4;
      if (at < n) onNal(bytes[at] & 0x1f, at);
      i = at + 1;
    } else if (isStart3) {
      const at = i + 3;
      if (at < n) onNal(bytes[at] & 0x1f, at);
      i = at + 1;
    } else {
      i += 1;
    }
  }
}

/** True when the access unit carries an IDR slice, i.e. a decodable keyframe. */
export function isKeyframe(bytes) {
  let key = false;
  forEachNalType(bytes, (type) => {
    if (type === NAL_IDR) key = true;
  });
  return key;
}

/**
 * The WebCodecs codec string (`avc1.PPCCLL`) for the access unit's SPS, or
 * `null` if it carries none. `PP`/`CC`/`LL` are the SPS profile_idc,
 * constraint flags, and level_idc — the three bytes after the SPS NAL header —
 * which is exactly what VideoDecoder.configure expects to select a profile.
 */
export function avcCodecString(bytes) {
  let codec = null;
  forEachNalType(bytes, (type, at) => {
    if (type !== NAL_SPS || codec !== null) return;
    if (at + 3 >= bytes.length) return;
    const hex = (v) => v.toString(16).padStart(2, "0");
    codec = `avc1.${hex(bytes[at + 1])}${hex(bytes[at + 2])}${hex(bytes[at + 3])}`;
  });
  return codec;
}

/**
 * Decodes one source's H.264 Annex-B stream to a canvas via WebCodecs. Fails
 * closed: constructs its VideoDecoder only once a keyframe with an SPS arrives
 * (so the profile is known), drops delta frames until then, and on any
 * unavailability (no WebCodecs, unsupported profile, decoder error) logs once
 * and stops — never throwing into the frame path.
 */
export class H264CanvasDecoder {
  constructor(target, log) {
    this.target = target;
    this.log = log;
    this.decoder = null;
    this.configured = false;
    this.failed = false;
    this.timestamp = 0;
  }

  /** Feeds one access unit (the codec payload). `isActive` gates the async
   *  paint so a frame decoded after session teardown is dropped. */
  decode(payload, isActive) {
    if (this.failed) return;
    if (typeof VideoDecoder === "undefined") {
      this.fail("WebCodecs VideoDecoder unavailable in this browser");
      return;
    }
    const keyframe = isKeyframe(payload);
    if (!this.configured) {
      if (!keyframe) return; // Cannot start mid-GOP; wait for a keyframe.
      if (!this.configure(payload, isActive)) return;
    }
    const type = keyframe ? "key" : "delta";
    this.timestamp += 1;
    try {
      this.decoder.decode(
        new EncodedVideoChunk({ type, timestamp: this.timestamp, data: payload }),
      );
    } catch (error) {
      this.fail(`H.264 decode failed: ${error}`);
    }
  }

  configure(keyframePayload, isActive) {
    const codec = avcCodecString(keyframePayload);
    if (!codec) {
      // A keyframe without an in-band SPS cannot name a profile; without a host
      // side avcC/config delivery there is nothing to configure against.
      this.fail("H.264 keyframe carries no SPS; cannot configure decoder");
      return false;
    }
    try {
      this.decoder = new VideoDecoder({
        output: (frame) => this.paint(frame, isActive),
        error: (error) => this.fail(`H.264 decoder error: ${error}`),
      });
      this.decoder.configure({ codec, optimizeForLatency: true });
    } catch (error) {
      this.fail(`H.264 decoder configure failed (codec ${codec}): ${error}`);
      return false;
    }
    this.configured = true;
    return true;
  }

  paint(frame, isActive) {
    try {
      if (!isActive()) return;
      const { canvas, ctx } = this.target;
      if (canvas.width !== frame.displayWidth || canvas.height !== frame.displayHeight) {
        canvas.width = frame.displayWidth;
        canvas.height = frame.displayHeight;
      }
      ctx.drawImage(frame, 0, 0);
    } finally {
      frame.close();
    }
  }

  fail(message) {
    if (this.failed) return;
    this.failed = true;
    this.log(`video codec H264 unavailable: ${message}`);
    if (this.decoder) {
      try {
        this.decoder.close();
      } catch {
        // A decoder that never configured cleanly may reject close; ignore.
      }
      this.decoder = null;
    }
  }
}
