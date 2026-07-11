// Display-pipeline health: failure latch, recovery, liveness watchdog, and
// the backend-owned failure page (DISP-01).
//
// A renderer or display-pipeline failure must never leave a valid-looking
// last successful image visible. Every failure carries a stable reason
// code; the failure page depends on nothing but raw Canvas2D calls, so it
// stays paintable when scene generation itself is broken.
//
// SIM / NOT FOR FLIGHT: this watchdog runs in the browser's own scheduling
// domains (requestAnimationFrame + setInterval) and is simulator-only. The
// health contract (`PanelHealth.snapshot()`) is shaped so a separately
// scheduled monitor can consume it later.

// Stable diagnostic reason codes, shown as `D-<code>` on the failure page.
// 1..99 mirror clients/web-instruments/src/render_status.rs exactly;
// 100+ are failures only this backend can observe. Codes are append-only:
// never reused or renumbered.
export const REASON = Object.freeze({
  OK: 0,
  // WASM-reported (render_status.rs).
  NOT_INITIALIZED: 1,
  CONTEXT_UNAVAILABLE: 2,
  STATE_TRUNCATED: 3,
  STATE_BAD_VERSION: 4,
  INVALID_PANEL: 5,
  SCENE_BUFFER_FULL: 6,
  SCENE_COMMAND_LIMIT: 7,
  SCENE_STRUCTURE: 8,
  // Backend-observed.
  WASM_LOAD: 100,
  ABI_MISMATCH: 101,
  INIT_FAILED: 102,
  RENDER_TRAP: 103,
  SCENE_FRAMING: 104,
  PAINT_FAILED: 105,
  LIVENESS: 106,
});

// A typed module-level fault raised by loadInstruments so callers can show
// the failure page with the precise reason instead of a generic error.
export class InstrumentFault extends Error {
  constructor(reason, message) {
    super(message);
    this.name = "InstrumentFault";
    this.reason = reason;
  }
}

// Per-panel display health: latches on any failure, recovers only after a
// sustained run of validated frames, and detects loss of frame advancement
// against an injected clock (no Date/performance dependency, so tests are
// deterministic).
export class PanelHealth {
  // livenessDeadlineMs: a panel whose successful-frame generation has not
  //   advanced for strictly longer than this is failed (LIVENESS).
  // recoveryFrames: consecutive validated frames required to clear a
  //   latched failure — one arbitrary good frame never clears it.
  constructor({ livenessDeadlineMs = 1000, recoveryFrames = 30 } = {}, nowMs = 0) {
    this.livenessDeadlineMs = livenessDeadlineMs;
    this.recoveryFrames = recoveryFrames;
    this.reset(nowMs);
  }

  // Explicit reinitialization transition: clears the latch and restarts
  // the liveness deadline. Only module reload/reinit paths call this.
  reset(nowMs) {
    this.latched = false;
    this.reason = REASON.OK;
    this.goodStreak = 0;
    this.lastGeneration = null;
    this.lastAdvanceMs = nowMs;
    this.counters = { failures: 0, duplicates: 0, recoveries: 0, livenessTrips: 0 };
  }

  // A validated frame was produced. Freshness credit requires the WASM
  // success generation to actually advance; a repeated generation is a
  // duplicate and earns nothing (it cannot feed the latch's recovery
  // streak nor reset the liveness deadline).
  reportSuccess(nowMs, generation) {
    const advanced = this.lastGeneration === null || generation !== this.lastGeneration;
    this.lastGeneration = generation;
    if (!advanced) {
      this.counters.duplicates += 1;
      return this.display();
    }
    this.lastAdvanceMs = nowMs;
    if (this.latched) {
      this.goodStreak += 1;
      if (this.goodStreak >= this.recoveryFrames) {
        this.latched = false;
        this.reason = REASON.OK;
        this.goodStreak = 0;
        this.counters.recoveries += 1;
      }
    }
    return this.display();
  }

  // Any render/validate/paint failure latches immediately.
  reportFailure(nowMs, reason) {
    this.latched = true;
    this.reason = reason;
    this.goodStreak = 0;
    this.counters.failures += 1;
    return this.display();
  }

  // Watchdog tick from a scheduling domain independent of the render
  // loop; latches LIVENESS when frame advancement stalls past the
  // deadline (strictly greater: an advance exactly at the deadline is
  // still on time).
  tick(nowMs) {
    if (nowMs - this.lastAdvanceMs > this.livenessDeadlineMs && !this.latched) {
      this.latched = true;
      this.reason = REASON.LIVENESS;
      this.goodStreak = 0;
      this.counters.livenessTrips += 1;
    }
    return this.display();
  }

  // What the compositor must show right now.
  display() {
    return this.latched
      ? { showFailure: true, reason: this.reason }
      : { showFailure: false, reason: REASON.OK };
  }

  // The health contract for a future separately scheduled monitor.
  snapshot() {
    return {
      latched: this.latched,
      reason: this.reason,
      goodStreak: this.goodStreak,
      lastGeneration: this.lastGeneration,
      lastAdvanceMs: this.lastAdvanceMs,
      counters: { ...this.counters },
    };
  }
}

// Paints the backend-owned failure page: covers ALL previous imagery (the
// full target, letterbox included) and shows an unmistakable DISPLAY FAIL
// with the stable diagnostic code. Uses only direct canvas calls — no
// scene generation, no WASM — so it works when those are the failure.
// Distinct from invalid aircraft data, which renders as in-scene red-X /
// flags through the normal pipeline.
export function drawFailurePage(ctx, width, height, reason) {
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.fillStyle = "#000";
  ctx.fillRect(0, 0, width, height);
  ctx.strokeStyle = "#f00";
  ctx.lineWidth = 4;
  ctx.strokeRect(2, 2, width - 4, height - 4);
  ctx.fillStyle = "#f00";
  ctx.font = "bold 28px system-ui, sans-serif";
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText("DISPLAY FAIL", width / 2, height / 2 - 14);
  ctx.font = "bold 16px system-ui, sans-serif";
  ctx.fillText(`D-${reason}`, width / 2, height / 2 + 16);
}
