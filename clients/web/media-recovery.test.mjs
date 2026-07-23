import assert from "node:assert/strict";

import { MEDIA_RESTART_AFTER_ATTEMPTS, MediaRecoveryGate } from "./media-recovery.js";

// ---- paced re-attach requests while all sources are stalled ----------------
{
  const gate = new MediaRecoveryGate(2000);
  assert.equal(gate.decide(false, 10_000), null, "healthy sources request nothing");
  assert.equal(gate.decide(true, 10_000), "attach", "an all-stalled tick requests attachment");
  assert.equal(gate.decide(true, 11_999), null, "requests are paced apart");
  assert.equal(gate.decide(true, 12_000), "attach");
}

// ---- a painted frame rearms the ladder from scratch ------------------------
{
  const gate = new MediaRecoveryGate(2000);
  assert.equal(gate.decide(true, 10_000), "attach");
  gate.notePaintedFrame();
  assert.equal(gate.decide(true, 10_001), "attach", "recovery clears the pacing window");
  gate.reset();
  assert.equal(gate.decide(true, 10_002), "attach", "a fresh session starts a fresh ladder");
}

// ---- unanswered attaches escalate to ONE session restart -------------------
{
  const gate = new MediaRecoveryGate(2000);
  let now = 10_000;
  for (let i = 0; i < MEDIA_RESTART_AFTER_ATTEMPTS; i += 1) {
    assert.equal(gate.decide(true, now), "attach", `attach attempt ${i + 1}`);
    now += 2000;
  }
  assert.equal(gate.decide(true, now), "restart", "the ladder ends in a session restart");
  assert.equal(gate.decide(true, now + 2000), null, "restart fires at most once");
  assert.equal(gate.decide(true, now + 60_000), null, "…no matter how long the stall lasts");

  // Frames painted on the REPLACEMENT session rearm the whole ladder.
  gate.notePaintedFrame();
  assert.equal(gate.decide(true, now + 62_000), "attach");
}

// ---- an intermittent stall never reaches the restart rung ------------------
{
  const gate = new MediaRecoveryGate(2000);
  let now = 10_000;
  for (let i = 0; i < MEDIA_RESTART_AFTER_ATTEMPTS * 3; i += 1) {
    assert.equal(gate.decide(true, now), "attach", "each stall episode re-attaches");
    gate.notePaintedFrame(); // the attach (or luck) brought frames back
    now += 5000;
  }
}

console.log("media recovery escalation policy passed");
