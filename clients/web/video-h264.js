// H.264 (Annex-B) decode groundwork for the viewer, behind the same capture
// identity gate as MJPEG (ADR-0016 codec FourCC dispatch). The host does not
// emit H.264 yet; this is the decode side of that path so a host that adopts it
// renders without a viewer change, and a browser lacking WebCodecs (or the
// stream's profile) fails visible with a typed reason rather than a blank
// canvas.
//
// The bitstream is decoded by the browser's WebCodecs VideoDecoder, not by hand
// in JS: this module only classifies Annex-B NAL units (to mark keyframes and
// derive the decoder config), feeds chunks, and blits the decoded frames — the
// transport/paint role the viewer keeps. Decoder ownership is bound to the
// transport session that created it (`isActive`); the caller closes and
// recreates it on session replacement, disconnect, capture discontinuity, or an
// in-band codec-config change, so a callback from a retired session can never
// govern a live one.

/** FourCC the host tags an H.264 Annex-B video body with (ADR-0016). */
export const FOURCC_H264 = "H264";

// H.264 NAL unit types read from the byte after each Annex-B start code
// (nal_unit_type = byte & 0x1F): 5 = IDR slice (keyframe), 7 = SPS, 8 = PPS.
const NAL_IDR = 5;
const NAL_SPS = 7;
const NAL_PPS = 8;

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

/** True when the access unit carries an IDR slice, i.e. a keyframe. An IDR
 *  slice alone is not decodable without its parameter sets; see
 *  [`hasParameterSets`]. */
export function isKeyframe(bytes) {
  let key = false;
  forEachNalType(bytes, (type) => {
    if (type === NAL_IDR) key = true;
  });
  return key;
}

/** True when the access unit carries BOTH parameter sets (SPS and PPS)
 *  in-band. A keyframe configures a decoder only when both are present:
 *  the SPS names the profile/level and the PPS the slice parameters, and
 *  WebCodecs cannot decode an Annex-B keyframe missing either. */
export function hasParameterSets(bytes) {
  let sps = false;
  let pps = false;
  forEachNalType(bytes, (type) => {
    if (type === NAL_SPS) sps = true;
    if (type === NAL_PPS) pps = true;
  });
  return sps && pps;
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
 * Decodes one source's H.264 Annex-B stream to a canvas via WebCodecs. Bound at
 * construction to the owning session's liveness (`options.isActive`); its output
 * callback tests that liveness, so once the caller replaces the decoder on a new
 * session the retired one can no longer paint. Fails visible: it configures only
 * once a keyframe carrying both parameter sets (SPS and PPS) arrives, drops delta
 * frames until then, reconfigures when a keyframe's codec string changes, and on
 * any unavailability
 * (no WebCodecs, unsupported profile, decoder error) marks a typed failed state,
 * leaves a marker on the canvas, and stops — never throwing into the frame path.
 *
 * `options`: `isActive` (owning session's liveness, default always-live),
 * `log` (diagnostics sink), and `VideoDecoder`/`EncodedVideoChunk` (injectable
 * for tests; default to the globals).
 */
export class H264CanvasDecoder {
  constructor(target, options = {}) {
    this.target = target;
    this.log = options.log ?? (() => {});
    this.isActive = options.isActive ?? (() => true);
    this.VideoDecoderCtor =
      options.VideoDecoder ?? (typeof VideoDecoder !== "undefined" ? VideoDecoder : undefined);
    this.EncodedChunkCtor =
      options.EncodedVideoChunk ??
      (typeof EncodedVideoChunk !== "undefined" ? EncodedVideoChunk : undefined);
    this.decoder = null;
    this.codecString = null;
    this.configured = false;
    this.failed = false;
    this.timestamp = 0;
  }

  /** Feeds one access unit (the codec payload). No token is captured here: the
   *  output callback tests the session liveness bound at construction. */
  decode(payload) {
    if (this.failed) return;
    if (!this.VideoDecoderCtor || !this.EncodedChunkCtor) {
      this.fail("WebCodecs VideoDecoder unavailable in this browser");
      return;
    }
    const keyframe = isKeyframe(payload);
    if (keyframe) {
      // A decodable keyframe carries both parameter sets in-band; require SPS
      // and PPS, and (re)configure when the SPS-derived codec string changes so
      // an in-band configuration change is honored.
      if (!hasParameterSets(payload)) {
        this.fail("H.264 keyframe missing in-band SPS/PPS; cannot configure decoder");
        return;
      }
      const codec = avcCodecString(payload);
      if (!codec) {
        this.fail("H.264 keyframe carries no SPS; cannot configure decoder");
        return;
      }
      if (codec !== this.codecString && !this.configure(codec)) {
        return;
      }
    }
    if (!this.configured) return; // Cannot start mid-GOP; wait for a keyframe.
    this.timestamp += 1;
    try {
      this.decoder.decode(
        new this.EncodedChunkCtor({
          type: keyframe ? "key" : "delta",
          timestamp: this.timestamp,
          data: payload,
        }),
      );
    } catch (error) {
      this.fail(`H.264 decode failed: ${error}`);
    }
  }

  configure(codec) {
    // Close the currently-held decoder, if any, before configuring a fresh one
    // for `codec`, so a codec change replaces it rather than leaking a stream.
    this.closeDecoder();
    try {
      this.decoder = new this.VideoDecoderCtor({
        output: (frame) => this.paint(frame),
        error: (error) => this.fail(`H.264 decoder error: ${error}`),
      });
      this.decoder.configure({ codec, optimizeForLatency: true });
    } catch (error) {
      this.fail(`H.264 decoder configure failed (codec ${codec}): ${error}`);
      return false;
    }
    this.codecString = codec;
    this.configured = true;
    return true;
  }

  paint(frame) {
    try {
      if (!this.isActive()) return;
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
    this.configured = false;
    this.log(`video codec H264 unavailable: ${message}`);
    this.paintFailure();
    this.closeDecoder();
  }

  paintFailure() {
    // Fail-visible: leave a typed marker on the canvas rather than a silent
    // blank, so an unsupported codec is diagnosable on screen. A headless or
    // fake target (tests) has no 2d context; the typed log is then the signal.
    try {
      const { canvas, ctx } = this.target;
      if (!canvas.width || !canvas.height) {
        canvas.width = 320;
        canvas.height = 240;
      }
      ctx.fillStyle = "#300";
      ctx.fillRect(0, 0, canvas.width, canvas.height);
      ctx.fillStyle = "#f66";
      ctx.font = "12px monospace";
      ctx.fillText("H.264 unavailable", 8, 20);
    } catch {
      // No paintable target; the log already carried the typed reason.
    }
  }

  /** Closes the decoder and its output stream. The caller invokes this on
   *  session replacement, disconnect, capture discontinuity, or teardown.
   *  Safe to call repeatedly. */
  close() {
    this.closeDecoder();
  }

  closeDecoder() {
    if (this.decoder) {
      try {
        this.decoder.close();
      } catch {
        // An unconfigured or already-closed decoder may reject close; ignore.
      }
      this.decoder = null;
    }
    this.configured = false;
  }
}

/**
 * Owns one H.264 decoder per video source, each bound to the transport session
 * that created it. This is the ownership boundary that keeps a decoder from
 * outliving its session: `for(sourceId, token)` hands back the decoder for the
 * current session; a different token (session replacement / reconnect) closes
 * the held decoder and builds one whose callbacks test the new session, so a
 * retired token can never govern a live session's frames. `reset(sourceId)`
 * drops one source's decoder (a capture discontinuity is a GOP boundary it
 * cannot span), and `closeAll()` empties the registry on session teardown.
 *
 * `makeDecoder(target, token)` builds a bound decoder; it is injected so the
 * ownership logic is testable without a real `VideoDecoder`.
 */
export class H264DecoderRegistry {
  constructor(makeDecoder) {
    this.makeDecoder = makeDecoder;
    this.entries = new Map();
  }

  for(sourceId, target, token) {
    const held = this.entries.get(sourceId);
    if (held && held.token === token) return held.decoder;
    if (held) held.decoder.close();
    const decoder = this.makeDecoder(target, token);
    this.entries.set(sourceId, { decoder, token });
    return decoder;
  }

  reset(sourceId) {
    const held = this.entries.get(sourceId);
    if (!held) return;
    held.decoder.close();
    this.entries.delete(sourceId);
  }

  closeAll() {
    for (const held of this.entries.values()) held.decoder.close();
    this.entries.clear();
  }
}
