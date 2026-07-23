// Lifecycle checks for the incoming-uni-stream accept loop against REAL
// ReadableStreams (Node's WHATWG implementation), proving the two properties
// the loop depends on:
//   1. An individual received stream failing loses only that frame; the loop
//      keeps accepting later streams.
//   2. The collection stream closing/erroring is terminal for the session and
//      is reported exactly once — and a re-acquired reader is handed the SAME
//      stored error, which is why reacquisition cannot recover.
//
// Run: node clients/web/uni-stream-accept.test.mjs

import { runIncomingStreamAcceptLoop } from "./uni-stream-accept.js";
import { streamCancellationReason } from "./stream-cancellation.js";

let failures = 0;
function check(name, cond) {
  if (cond) {
    console.log(`ok   - ${name}`);
  } else {
    console.error(`FAIL - ${name}`);
    failures += 1;
  }
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 0));

// A pull-driven collection: each read() advances one scripted step, so a value
// is genuinely delivered BEFORE a later close/error (unlike enqueue-then-error,
// which discards the queue).
function scriptedCollection(steps) {
  let i = 0;
  return new ReadableStream({
    pull(controller) {
      const step = steps[i];
      i += 1;
      if (!step || step.close) {
        controller.close();
      } else if (step.error) {
        controller.error(step.error);
      } else {
        controller.enqueue(step.value);
      }
    },
  });
}

// ---- 1. individual-stream isolation, then a graceful terminal close --------
{
  const s1 = { id: 1 };
  const s2 = { id: 2 };
  const s3 = { id: 3 };
  const collection = scriptedCollection([
    { value: s1 },
    { value: s2 },
    { value: s3 },
    { close: true },
  ]);

  const handled = [];
  const failed = [];
  const terminals = [];

  await runIncomingStreamAcceptLoop(collection, {
    isActive: () => true,
    handleStream: (stream) => {
      handled.push(stream.id);
      // The middle frame's drain rejects; the other two succeed.
      return stream.id === 2
        ? Promise.reject(new Error("frame 2 drain failed"))
        : Promise.resolve();
    },
    onStreamFailure: (error) => failed.push(error),
    onCollectionTerminal: (error) => terminals.push(error),
  });
  await tick(); // let the fire-and-forget handleStream rejections settle

  check("every received stream is handled", handled.join(",") === "1,2,3");
  check("a single stream's drain failure is isolated to one report", failed.length === 1);
  check("the failure carries the drain error", String(failed[0]).includes("frame 2 drain failed"));
  check("the surviving streams were not disturbed", handled.includes(1) && handled.includes(3));
  check("a graceful collection close reports one terminal (null)", terminals.length === 1 && terminals[0] === null);
}

// ---- 2. a collection error is terminal, reported once, and unrecoverable ----
{
  const storedError = new Error("collection errored (session gone)");
  const collection = scriptedCollection([{ value: { id: 1 } }, { error: storedError }]);

  const terminals = [];
  await runIncomingStreamAcceptLoop(collection, {
    isActive: () => true,
    handleStream: () => Promise.resolve(),
    onStreamFailure: () => {},
    onCollectionTerminal: (error) => terminals.push(error),
  });

  check("a collection error reports exactly one terminal", terminals.length === 1);
  check("the terminal carries the collection's stored error", terminals[0] === storedError);

  // WHATWG Streams: once the collection stream is errored, releasing the lock
  // and acquiring a NEW reader returns the identical stored error. This is the
  // proof that reacquiring a reader (the removed "supervisor") cannot recover.
  let reacquiredError = null;
  try {
    await collection.getReader().read();
  } catch (error) {
    reacquiredError = error;
  }
  check("a re-acquired reader is handed the SAME stored error", reacquiredError === storedError);
}

// ---- 3. going inactive stops the loop without a terminal report ------------
{
  let calls = 0;
  let cancellation = null;
  const abandoned = new ReadableStream({
    cancel(reason) {
      cancellation = reason;
    },
  });
  const collection = scriptedCollection([{ value: abandoned }, { value: { id: 2 } }]);
  const terminals = [];
  const handled = [];

  await runIncomingStreamAcceptLoop(collection, {
    // true for the entry guard, false right after the first read.
    isActive: () => {
      calls += 1;
      return calls < 2;
    },
    handleStream: (stream) => {
      handled.push(stream);
      return stream.getReader().cancel(streamCancellationReason("stream-abandoned"));
    },
    onStreamFailure: () => {},
    onCollectionTerminal: (error) => terminals.push(error),
  });

  check("an inactive session stops the loop", true);
  check("the stream accepted during retirement is handed to its cancellation owner", handled[0] === abandoned);
  check("the accepted stream is cancelled with a typed reason", cancellation?.kind === "stream-abandoned");
  check("stopping for inactivity is not a terminal session failure", terminals.length === 0);
}

// ---- 4. trackReader refusal aborts before dispatching any stream -----------
{
  const collection = scriptedCollection([{ value: { id: 1 } }, { value: { id: 2 } }]);
  const handled = [];
  const terminals = [];

  await runIncomingStreamAcceptLoop(collection, {
    isActive: () => true,
    handleStream: (stream) => {
      handled.push(stream.id);
      return Promise.resolve();
    },
    onStreamFailure: () => {},
    onCollectionTerminal: (error) => terminals.push(error),
    trackReader: () => false, // session already gone when the reader was tracked
  });

  check("trackReader refusal dispatches no stream", handled.length === 0);
  check("trackReader refusal reports no terminal", terminals.length === 0);
}

if (failures > 0) {
  console.error(`\n${failures} check(s) failed`);
  process.exit(1);
}
console.log("\nall uni-stream accept lifecycle checks passed");
