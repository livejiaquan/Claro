import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { DownloadUiState } from "../downloadState";
import type { DownloadProgress } from "../types";
import DownloadStatus from "./DownloadStatus";

afterEach(cleanup);

const activeProgress: DownloadProgress = {
  model_id: "model-a",
  downloaded_mb: 25,
  total_mb: 100,
  done: false,
  downloaded: false,
  activation_status: "none",
  error: null,
};

const state = (overrides: Partial<DownloadUiState> = {}): DownloadUiState => ({
  phase: "downloading",
  active: true,
  canStart: false,
  canCancel: true,
  percent: 25,
  progress: activeProgress,
  error: null,
  ...overrides,
});

describe("DownloadStatus", () => {
  it("exposes determinate progress and a named cancel action", () => {
    const onCancel = vi.fn();
    render(
      <DownloadStatus
        state={state()}
        label="語音模型"
        onCancel={onCancel}
      />,
    );

    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe("25");
    const cancel = screen.getByRole("button", { name: "取消 語音模型下載" });
    expect(cancel.hasAttribute("disabled")).toBe(false);
    fireEvent.click(cancel);
    expect(onCancel).toHaveBeenCalledOnce();
  });

  it("disables repeated cancellation and announces the pending state", () => {
    render(
      <DownloadStatus
        state={state({ phase: "cancelling", canCancel: false })}
        label="語音模型"
        onCancel={() => {}}
      />,
    );

    expect(screen.getByRole("button", { name: "取消 語音模型下載" }).hasAttribute("disabled")).toBe(true);
    expect(screen.getByText("正在取消…")).toBeTruthy();
  });

  it("uses an alert for recoverable failures", () => {
    render(
      <DownloadStatus
        state={state({
          phase: "failed",
          active: false,
          canStart: true,
          canCancel: false,
          percent: null,
          progress: null,
          error: "連線中斷",
        })}
        label="語音模型"
        onCancel={() => {}}
      />,
    );

    expect(screen.getByRole("alert").textContent).toContain("下載失敗：連線中斷");
    expect(screen.queryByRole("progressbar")).toBeNull();
  });

  it("announces user cancellation without presenting it as an error", () => {
    render(
      <DownloadStatus
        state={state({
          phase: "cancelled",
          active: false,
          canStart: true,
          canCancel: false,
          percent: null,
          progress: null,
        })}
        label="語音模型"
        onCancel={() => {}}
      />,
    );

    expect(screen.getByRole("status").textContent).toContain("已取消下載");
    expect(screen.queryByRole("alert")).toBeNull();
  });
});
