// H.264 (Annex-B) decode session layer for the viewer, behind the same
// capture identity gate as MJPEG (ADR-0016 codec FourCC dispatch). The host
// does not emit H.264 yet; this is the decode side of that path so a host
// that adopts it renders without a viewer change, and a browser lacking
// WebCodecs (or the stream's profile) fails visible with a typed reason
// rather than a blank canvas.
//
// The bitstream is decoded by the browser's WebCodecs VideoDecoder, and what
// a chunk MEANS (keyframe, in-band SPS/PPS, codec string) is classified by
// the shared Rust wasm export `classifyH264Chunk` — the same
// pilotage_protocol::h264 definitions every consumer uses, so NAL-structure
// rules can never drift into hand-written JS. This module keeps only the
// platform-API session layer: decoder ownership bound to the transport
// session that created it (`isActive`), configure/feed, and paint; the
// caller closes and recreates a decoder on session replacement, disconnect,
// capture discontinuity, or an in-band codec-config change, so a callback
// from a retired session can never govern a live one.

import { classifyH264Chunk } from "./instrument-runtime.js";

/** FourCC the host tags an H.264 Annex-B video body with (ADR-0016). */
export const FOURCC_H264 = "H264";

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
 * `log` (diagnostics sink), `classify` (chunk classifier; defaults to the
 * shared wasm export), and `VideoDecoder`/`EncodedVideoChunk` (injectable
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
    this.classify = options.classify ?? classifyH264Chunk;
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
    const cls = this.classify(payload);
    if (cls.kind === "undecodable-keyframe") {
      // A keyframe that cannot configure a decoder (missing SPS/PPS, or an
      // SPS too short to name a profile) fails visible with the typed reason.
      this.fail(`H.264 keyframe cannot configure a decoder: ${cls.reason}`);
      return;
    }
    const keyframe = cls.kind === "keyframe";
    // (Re)configure when a keyframe's SPS-derived codec string changes, so an
    // in-band configuration change is honored.
    if (keyframe && cls.codec !== this.codecString && !this.configure(cls.codec)) {
      return;
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
