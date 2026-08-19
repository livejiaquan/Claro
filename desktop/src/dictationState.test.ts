import { describe, expect, it } from "vitest";
import { visiblePendingResult } from "./dictationState";
import type { DictationEvent, PendingResult } from "./types";

const pending: PendingResult = {
  raw: "上一段原文",
  text: "上一段保留文字",
  reason: "paste_failed",
};

const finished = (overrides: Partial<DictationEvent> = {}): DictationEvent => ({
  session: 2,
  phase: "finished",
  outcome: "stt_failed",
  recovery_available: false,
  ...overrides,
});

describe("visiblePendingResult", () => {
  it("does not attach an older recovery result to a new STT failure", () => {
    expect(visiblePendingResult(pending, finished())).toBeNull();
  });

  it("shows the result when the current failure explicitly preserves it", () => {
    expect(
      visiblePendingResult(
        pending,
        finished({ outcome: "paste_failed", recovery_available: true }),
      ),
    ).toBe(pending);
  });

  it("hides recovery while a new dictation is processing", () => {
    expect(visiblePendingResult(pending, { session: 3, phase: "processing", outcome: null, recovery_available: false })).toBeNull();
  });
});
