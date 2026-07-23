import { describe, expect, it } from "vitest";
import { resolveDownloadState } from "./downloadState";
import type { DownloadProgress } from "./types";

const progress = (overrides: Partial<DownloadProgress> = {}): DownloadProgress => ({
  model_id: "model-a",
  downloaded_mb: 25,
  total_mb: 100,
  done: false,
  downloaded: false,
  activation_status: "none",
  error: null,
  ...overrides,
});

describe("resolveDownloadState", () => {
  it("lets a failure event override a stale backend downloading flag", () => {
    const state = resolveDownloadState({
      modelId: "model-a",
      downloaded: false,
      backendDownloading: true,
      progress: progress({ error: "連線中斷" }),
    });

    expect(state.phase).toBe("failed");
    expect(state.active).toBe(false);
    expect(state.canStart).toBe(true);
  });

  it("treats user cancellation as a recoverable status instead of a failure", () => {
    const state = resolveDownloadState({
      modelId: "model-a",
      downloaded: false,
      backendDownloading: true,
      progress: progress({ error: "下載已取消" }),
    });

    expect(state.phase).toBe("cancelled");
    expect(state.error).toBeNull();
    expect(state.canStart).toBe(true);
  });

  it("lets a completion event override a stale backend downloading flag", () => {
    const state = resolveDownloadState({
      modelId: "model-a",
      downloaded: false,
      backendDownloading: true,
      progress: progress({ done: true, downloaded: true }),
    });

    expect(state.phase).toBe("complete");
    expect(state.active).toBe(false);
  });

  it("shows preparing before the first progress event", () => {
    const state = resolveDownloadState({
      modelId: "model-a",
      downloaded: false,
      backendDownloading: true,
      progress: null,
    });

    expect(state.phase).toBe("preparing");
    expect(state.canCancel).toBe(true);
  });

  it("shows preparing immediately after the user starts a download", () => {
    const state = resolveDownloadState({
      modelId: "model-a",
      downloaded: false,
      backendDownloading: false,
      progress: null,
      startRequested: true,
    });

    expect(state.phase).toBe("preparing");
    expect(state.canStart).toBe(false);
  });

  it("reports determinate progress when total size is known", () => {
    const state = resolveDownloadState({
      modelId: "model-a",
      downloaded: false,
      backendDownloading: true,
      progress: progress(),
    });

    expect(state.phase).toBe("downloading");
    expect(state.percent).toBe(25);
  });

  it("disables repeated cancellation while cancellation is pending", () => {
    const state = resolveDownloadState({
      modelId: "model-a",
      downloaded: false,
      backendDownloading: true,
      progress: progress(),
      cancelRequested: true,
    });

    expect(state.phase).toBe("cancelling");
    expect(state.canCancel).toBe(false);
  });

  it("ignores another model's progress", () => {
    const state = resolveDownloadState({
      modelId: "model-b",
      downloaded: false,
      backendDownloading: false,
      progress: progress(),
    });

    expect(state.phase).toBe("idle");
    expect(state.canStart).toBe(true);
  });

  it("keeps a downloaded model complete even if a local start marker is stale", () => {
    const state = resolveDownloadState({
      modelId: "model-a",
      downloaded: true,
      backendDownloading: false,
      progress: null,
      startRequested: true,
    });

    expect(state.phase).toBe("complete");
    expect(state.active).toBe(false);
  });
});
