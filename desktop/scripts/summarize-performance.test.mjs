import test from "node:test";
import assert from "node:assert/strict";
import { summarizeHistory } from "./summarize-performance.mjs";

test("summarizes release-to-paste latency without exposing transcript text", () => {
  const content = [
    JSON.stringify({
      status: "pasted",
      raw: "不應出現在摘要",
      text: "也不應出現在摘要",
      timings: {
        release_to_paste_ms: 900,
        stt_ms: 600,
        polish_ms: null,
        focus_guard_ms: 20,
        inject_ms: 355,
      },
      polish: { mode: "raw" },
    }),
    JSON.stringify({
      status: "pasted",
      timings: { release_to_paste_ms: 3100, stt_ms: 800, polish_ms: 1800 },
      polish: { mode: "clean" },
    }),
    JSON.stringify({
      status: "cancelled",
      timings: { release_to_paste_ms: 100 },
      polish: { mode: "raw" },
    }),
    "{broken",
  ].join("\n");

  const summary = summarizeHistory(content);

  assert.equal(summary.pasted_entries, 2);
  assert.equal(summary.entries_with_release_to_paste, 2);
  assert.equal(summary.corrupt_lines, 1);
  assert.deepEqual(summary.release_to_paste.raw, {
    samples: 1,
    p50_ms: 900,
    p95_ms: 900,
    max_ms: 900,
  });
  assert.deepEqual(summary.release_to_paste.polished, {
    samples: 1,
    p50_ms: 3100,
    p95_ms: 3100,
    max_ms: 3100,
  });
  assert.deepEqual(summary.stages.focus_guard, {
    samples: 1,
    p50_ms: 20,
    p95_ms: 20,
    max_ms: 20,
  });
  assert.deepEqual(summary.stages.inject, {
    samples: 1,
    p50_ms: 355,
    p95_ms: 355,
    max_ms: 355,
  });
  assert.equal("raw" in summary, false);
  assert.equal("text" in summary, false);
});
