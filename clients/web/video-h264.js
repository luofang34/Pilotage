// H.264 (Annex-B) platform adapter for the viewer, behind the same capture
// identity gate as MJPEG (ADR-0016 codec FourCC dispatch). A host tagging a
// video body with the H264 FourCC renders here without a viewer change, and
// a browser lacking WebCodecs (or the stream's profile) fails visible with a
// typed reason rather than a blank canvas.
//
// Every decision lives in shared Rust (pilotage_protocol::h264, via the wasm
// exports): what a chunk means, the decode-session state machine
// (configure/feed/drop/fail), and per-source decoder ownership by session
// token. This module is the thin layer the browser platform forces: it
// executes the returned actions against WebCodecs `VideoDecoder`, reports
// platform failures back into the session machine, and blits decoded frames —
// there is no second parser, validator, or ownership state machine in JS.

import { H264DecodeSession, H264SourceOwnership } from "./instrument-runtime.js";

/** FourCC the host tags an H.264 Annex-B video body with (ADR-0016). */
export const FOURCC_H264 = "H264";

/**
 * Executes decode-session actions for one source against WebCodecs, painting
 * to a canvas. Bound at construction to the owning session's liveness
 * (`options.isActive`); the paint callback tests that liveness, so once the
 * caller replaces the decoder on a new session the retired one can no longer
 * paint. All decode decisions come from the wasm session machine, and every
 * platform callback carries the decoder GENERATION it was configured under:
 * the machine honors it only while that generation is current, so a callback
 * from a replaced, reset, failed, or retired decoder can never paint over
 * its successor or the failure marker. A platform failure (no WebCodecs,
 * configure error, decode throw, the asynchronous error callback) is
 * reported back with `platformFailed()` and surfaced once with a typed
 * reason and a fail-visible canvas marker.
 *
 * `options`: `isActive` (owning session's liveness, default always-live),
 * `log` (diagnostics sink), `session` (decode-session machine; defaults to a
 * fresh wasm `H264DecodeSession`), and `VideoDecoder`/`EncodedVideoChunk`
 * (injectable for tests; default to the globals).
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
    this.session = options.session ?? new H264DecodeSession();
    this.decoder = null;
    this.failReported = false;
    this.timestamp = 0;
  }

  /** Session-failure surface for callers and tests; the state itself lives
   *  in the wasm session machine. */
  get failed() {
    return this.session.failed;
  }

  /** Feeds one access unit (the codec payload) through the session machine
   *  and executes its decision. */
  decode(payload) {
    const step = this.session.onChunk(payload);
    if (step.action === "drop") return;
    if (step.action === "fail") {
      this.fail(`H.264 keyframe cannot configure a decoder: ${step.reason}`);
      return;
    }
    if (!this.VideoDecoderCtor || !this.EncodedChunkCtor) {
      this.session.platformFailed();
      this.fail("WebCodecs VideoDecoder unavailable in this browser");
      return;
    }
    if (step.action === "configure-and-feed" && !this.configure(step.codec)) {
      return;
    }
    this.timestamp += 1;
    try {
      this.decoder.decode(
        new this.EncodedChunkCtor({
          type: step.keyframe ? "key" : "delta",
          timestamp: this.timestamp,
          data: payload,
        }),
      );
    } catch (error) {
      this.session.platformFailed();
      this.fail(`H.264 decode failed: ${error}`);
    }
  }

  configure(codec) {
    // Close the currently-held decoder, if any, before configuring a fresh one
    // for `codec`, so a codec change replaces it rather than leaking a stream.
    this.closeDecoder();
    // Every callback of this platform decoder carries the generation the
    // session machine assigned to this configuration; the machine refuses
    // output and errors from any superseded generation.
    const generation = this.session.generation;
    try {
      this.decoder = new this.VideoDecoderCtor({
        output: (frame) => this.paint(frame, generation),
        error: (error) => {
          if (!this.session.acceptsOutputFrom(generation)) return;
          this.session.platformFailed();
          this.fail(`H.264 decoder error: ${error}`);
        },
      });
      this.decoder.configure({ codec, optimizeForLatency: true });
    } catch (error) {
      this.session.platformFailed();
      this.fail(`H.264 decoder configure failed (codec ${codec}): ${error}`);
      return false;
    }
    return true;
  }

  paint(frame, generation) {
    try {
      if (!this.session.acceptsOutputFrom(generation) || !this.isActive()) return;
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

  /** Surfaces one typed failure (log + fail-visible marker) and closes the
   *  platform decoder. The session machine already refuses further feeds;
   *  this only guards duplicate reporting. */
  fail(message) {
    if (this.failReported) return;
    this.failReported = true;
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

  /** Retires the session machine and closes the platform decoder. The
   *  caller invokes this on session replacement, disconnect, capture
   *  discontinuity, or teardown; retirement is permanent, so any callback
   *  a superseded decoder still holds is refused. Safe to call
   *  repeatedly. */
  close() {
    this.session.retire();
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
  }
}

/**
 * Holds one H.264 decoder object per video source and executes the ownership
 * transitions decided by the shared Rust machine (`H264SourceOwnership`,
 * keyed by the transport session token's numeric `generation`): `reuse`
 * hands back the held decoder; `replace` closes the retired decoder and
 * builds one whose callbacks test the new session, so a retired token can
 * never govern a live session's frames. `reset(sourceId)` drops one source's
 * decoder (a capture discontinuity is a GOP boundary it cannot span), and
 * `closeAll()` empties the registry on session teardown.
 *
 * `makeDecoder(target, token)` builds a bound decoder; it is injected so the
 * platform layer is testable without a real `VideoDecoder`.
 */
export class H264DecoderRegistry {
  constructor(makeDecoder, ownership = new H264SourceOwnership()) {
    this.makeDecoder = makeDecoder;
    this.ownership = ownership;
    this.decoders = new Map();
  }

  for(sourceId, target, token) {
    const claim = this.ownership.claim(sourceId, token?.generation ?? 0);
    if (claim === "reuse") return this.decoders.get(sourceId);
    if (claim === "replace") this.decoders.get(sourceId)?.close();
    const decoder = this.makeDecoder(target, token);
    this.decoders.set(sourceId, decoder);
    return decoder;
  }

  reset(sourceId) {
    if (!this.ownership.reset(sourceId)) return;
    this.decoders.get(sourceId)?.close();
    this.decoders.delete(sourceId);
  }

  closeAll() {
    this.ownership.clear();
    for (const decoder of this.decoders.values()) decoder.close();
    this.decoders.clear();
  }
}
