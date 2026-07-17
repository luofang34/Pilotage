// Display-pipeline health: failure latch, recovery, liveness watchdog, and
// the backend-owned failure page.
//
// A renderer or display-pipeline failure must never leave a valid-looking
// last successful image visible. Every failure carries a stable reason
// code; an independent DOM surface contains Canvas2D failures while the
// canvas failure page remains available when scene generation is broken.
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
  SCENE_LAYER_CONTRACT: 9,
  SCENE_CRITICAL_LAYERS_MISSING: 10,
  // Backend-observed.
  WASM_LOAD: 100,
  ABI_MISMATCH: 101,
  INIT_FAILED: 102,
  RENDER_TRAP: 103,
  SCENE_FRAMING: 104,
  PAINT_FAILED: 105,
  LIVENESS: 106,
  STATE_WRITE_FAILED: 107,
  GLYPH_ASSET: 108,
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

export function createDomFaultPresenter(canvas) {
  const parent = canvas.parentElement;
  if (!parent) throw new Error("instrument canvas has no fault-presenter parent");
  const element = document.createElement("div");
  element.setAttribute("role", "alert");
  element.setAttribute("aria-live", "assertive");
  element.setAttribute("aria-hidden", "true");
  Object.assign(element.style, {
    position: "absolute",
    inset: "0",
    zIndex: "10",
    display: "none",
    placeContent: "center",
    boxSizing: "border-box",
    border: "4px solid #f00",
    background: "#000",
    color: "#f00",
    font: "bold 24px system-ui, sans-serif",
    textAlign: "center",
    whiteSpace: "pre-line",
    pointerEvents: "none",
  });
  parent.insertBefore(element, canvas.nextSibling);
  return {
    show(reason) {
      element.textContent = `DISPLAY FAIL\nD-${reason}\nSIM / NOT FOR FLIGHT`;
      element.style.display = "grid";
      element.setAttribute("aria-hidden", "false");
    },
    hide() {
      element.style.display = "none";
      element.setAttribute("aria-hidden", "true");
    },
  };
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
  // tickIntervalMs: the cadence the caller schedules `tick` at. A tick
  //   arriving later than twice this cadence proves the watchdog's own
  //   scheduling domain was starved (browser throttling of an occluded
  //   or backgrounded page suspends rAF and clamps timers together), so
  //   that tick re-arms the deadline instead of judging liveness — a
  //   deadline measured by a clock that itself stopped says nothing
  //   about the render pipeline, and latching from it makes panels
  //   blink FAIL/OK with every compositor burst. A genuinely dead
  //   render loop on a normally scheduled page still trips within one
  //   deadline of ticks running on cadence — including within one
  //   deadline of scheduling resuming, which is exactly when a viewer
  //   is looking again.
  constructor(
    { livenessDeadlineMs = 1000, recoveryFrames = 30, tickIntervalMs = 250 } = {},
    nowMs = 0,
  ) {
    this.livenessDeadlineMs = livenessDeadlineMs;
    this.recoveryFrames = recoveryFrames;
    this.tickIntervalMs = tickIntervalMs;
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
    // Seed the tick clock at reset so the FIRST tick has a cadence
    // baseline: a first tick already late against the scheduled cadence
    // (the page was backgrounded before the watchdog ever ran) is
    // recognized as starvation, not read as a dead renderer. Without
    // this the first tick could never detect starvation and would falsely
    // latch LIVENESS on a delayed initial fire.
    this.lastTickMs = nowMs;
    this.counters = {
      failures: 0,
      duplicates: 0,
      recoveries: 0,
      livenessTrips: 0,
      starvedTicks: 0,
    };
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
  // still on time). A tick whose own arrival is late against
  // `tickIntervalMs` proves page-wide scheduling starvation and re-arms
  // the deadline instead of judging with a stalled clock (see the
  // constructor's tickIntervalMs contract).
  tick(nowMs) {
    // lastTickMs is seeded at reset, so even the first tick has a cadence
    // baseline to measure its own lateness against.
    const starved = nowMs - this.lastTickMs > 2 * this.tickIntervalMs;
    this.lastTickMs = nowMs;
    if (starved) {
      this.lastAdvanceMs = nowMs;
      this.counters.starvedTicks += 1;
      return this.display();
    }
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

// Paints the canvas failure page without scene generation or WASM. An
// independent fault presenter remains necessary because the Canvas2D
// backend itself can be the failed component.
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

function showIndependentFailure(presenter, reason) {
  try {
    presenter?.show(reason);
    return presenter != null;
  } catch {
    return false;
  }
}

function hideIndependentFailure(presenter) {
  try {
    presenter?.hide();
    return true;
  } catch {
    return false;
  }
}

function coverFailure(ctx, width, height, presenter, reason) {
  const independentlyCovered = showIndependentFailure(presenter, reason);
  let canvasCovered = false;
  try {
    drawFailurePage(ctx, width, height, reason);
    canvasCovered = true;
  } catch {
    // The independent presenter is the containment path for Canvas failure.
  }
  return independentlyCovered || canvasCovered;
}

function failTarget(health, target, nowMs, reason) {
  const [panel, ctx, canvas, presenter] = target;
  const display = health[panel].reportFailure(nowMs, reason);
  return {
    panel,
    ok: false,
    reason,
    covered: coverFailure(ctx, canvas.width, canvas.height, presenter, display.reason),
  };
}

export function failInstrumentSet(health, targets, nowMs, reason) {
  return targets.map((target) => failTarget(health, target, nowMs, reason));
}

export function coverInstrumentFailures(health, targets) {
  return targets.map(([panel, ctx, canvas, presenter]) => {
    const display = health[panel].display();
    return {
      panel,
      showFailure: display.showFailure,
      covered:
        display.showFailure &&
        coverFailure(ctx, canvas.width, canvas.height, presenter, display.reason),
    };
  });
}

export function renderInstrumentSet(module, health, targets, state, nowMs) {
  let stateResult;
  try {
    stateResult = module.writeState(state);
  } catch {
    stateResult = { ok: false, reason: REASON.STATE_WRITE_FAILED };
  }
  if (stateResult?.ok !== true) {
    return failInstrumentSet(
      health,
      targets,
      nowMs,
      stateResult?.reason ?? REASON.STATE_WRITE_FAILED,
    );
  }

  // One alert step per frame, before any panel renders: every panel in
  // this set then draws from the same manager output. The independent
  // watchdog verdict (any panel currently latched failed) feeds the
  // manager's path-health input. A step failure fails the whole set —
  // a wasm anomaly here is as disqualifying as a state-write failure.
  let alertResult;
  try {
    const pathHealthy = targets.every(([panel]) => !health[panel].display().showFailure);
    alertResult = module.stepAlerts(nowMs, pathHealthy);
  } catch {
    alertResult = { ok: false, reason: REASON.RENDER_TRAP };
  }
  if (alertResult?.ok !== true) {
    return failInstrumentSet(health, targets, nowMs, alertResult?.reason ?? REASON.RENDER_TRAP);
  }

  const outcomes = [];
  for (const target of targets) {
    const [panel, ctx, canvas, presenter] = target;
    let result;
    try {
      result = module.renderPanel(panel, ctx, canvas.width, canvas.height);
    } catch {
      result = { ok: false, reason: REASON.RENDER_TRAP };
    }
    if (result?.ok !== true) {
      outcomes.push(failTarget(health, target, nowMs, result?.reason ?? REASON.RENDER_TRAP));
      continue;
    }
    const display = health[panel].reportSuccess(nowMs, result.generation);
    outcomes.push({
      panel,
      ok: true,
      generation: result.generation,
      showFailure: display.showFailure,
      covered:
        display.showFailure &&
        coverFailure(ctx, canvas.width, canvas.height, presenter, display.reason),
      faultCleared: !display.showFailure && hideIndependentFailure(presenter),
    });
  }
  return outcomes;
}

export function tickInstrumentSet(health, targets, nowMs) {
  return targets.map(([panel, ctx, canvas, presenter]) => {
    const display = health[panel].tick(nowMs);
    return {
      panel,
      showFailure: display.showFailure,
      covered:
        display.showFailure &&
        coverFailure(ctx, canvas.width, canvas.height, presenter, display.reason),
    };
  });
}

export function startDisplayLoop(requestFrame, renderFrame, reportFailure) {
  const loop = (nowMs) => {
    try {
      renderFrame(nowMs);
    } catch {
      try {
        reportFailure(nowMs);
      } catch {
        // Frame scheduling must survive failure-reporting faults.
      }
    } finally {
      requestFrame(loop);
    }
  };
  requestFrame(loop);
}
